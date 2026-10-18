//! Action items and decisions, extracted from a meeting's transcript and stored
//! as structured records in `meeting_items`.
//!
//! Grounding rules (from the product spec):
//! - Every item carries a verbatim `evidence_quote` from the transcript.
//! - The quote is checked against the transcript; the timestamp is taken from
//!   where the quote actually appears, never from the model.
//! - Owners and due dates are left empty unless the transcript states them.
//! - Re-running extraction never overwrites items the user created or edited.

use crate::state::AppState;
use crate::summary::llm_client::{generate_summary, LLMProvider};
use crate::summary::processor::{chunk_text, rough_token_count};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

// Grounding and storage live in hearken-core so hearken-mcp (Claude) follows the same rules.
pub use hearken_core::items::{
    format_clock, ground_items, insert_item, load_timed_transcript, locate_quote, render_transcript,
    replace_ai_items, MeetingItem, RawItem, TimedLine,
};
use hearken_core::items::clean_opt;

/// Upper bound for one extraction prompt, even on large-context cloud models,
/// so long meetings are processed in focused passes.
const MAX_EXTRACTION_CHUNK_TOKENS: usize = 6000;

/// An open action item together with the meeting it came from (for the cross-meeting list).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct OpenActionItem {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub item: MeetingItem,
    pub meeting_title: String,
    pub meeting_created_at: String,
}

// ---------------------------------------------------------------------------
// Transcript preparation
// ---------------------------------------------------------------------------
// LLM extraction
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
struct ExtractionResponse {
    #[serde(default)]
    action_items: Vec<RawItem>,
    #[serde(default)]
    decisions: Vec<RawItem>,
}

const SYSTEM_PROMPT: &str = "You extract commitments and decisions from meeting transcripts. \
You output only valid JSON and never invent information.";

fn build_prompt(transcript_chunk: &str) -> String {
    format!(
        r#"Read the meeting transcript below. Each line starts with its time in the recording.

Extract:
1. action_items: concrete tasks someone committed to or was asked to do.
2. decisions: things the group agreed or decided.

Rules:
- Use only what is in the transcript. Do not guess.
- "owner": the person responsible, exactly as named in the transcript. Use null if unclear.
- "due": the deadline as spoken (e.g. "Friday", "end of month"). Use null if none was stated.
- "priority": "high", "medium" or "low" only if the urgency is clear, otherwise null.
- "evidence_quote": copy 5 to 25 consecutive words EXACTLY from one transcript line that supports the item. Do not include the [time] prefix.
- If there are none, return empty arrays.

Return only this JSON, with no other text:
{{"action_items":[{{"title":"","description":null,"owner":null,"due":null,"priority":null,"evidence_quote":""}}],"decisions":[{{"title":"","description":null,"owner":null,"due":null,"priority":null,"evidence_quote":""}}]}}

<transcript>
{transcript_chunk}
</transcript>"#
    )
}

/// Pulls the JSON object out of a model reply (tolerates code fences and
/// reasoning text around it).
fn parse_response(raw: &str) -> Option<ExtractionResponse> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&raw[start..=end]).ok()
}

/// Everything needed to call the user's configured model.
pub struct LlmTarget<'a> {
    pub provider: &'a LLMProvider,
    pub model_name: &'a str,
    pub api_key: &'a str,
    pub ollama_endpoint: Option<&'a str>,
    pub custom_openai_endpoint: Option<&'a str>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub app_data_dir: Option<&'a PathBuf>,
    pub token_threshold: usize,
}

/// Extracts items for a meeting and replaces its previous AI-generated items.
/// User-created and user-edited items are kept. Returns how many items were stored.
pub async fn extract_and_store(
    pool: &SqlitePool,
    meeting_id: &str,
    llm: &LlmTarget<'_>,
    cancellation_token: Option<&CancellationToken>,
) -> Result<usize, String> {
    let lines = load_timed_transcript(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to load transcript: {}", e))?;
    if lines.is_empty() {
        return Ok(0);
    }

    let transcript = render_transcript(&lines);
    let chunk_tokens = llm
        .token_threshold
        .saturating_sub(800)
        .clamp(1000, MAX_EXTRACTION_CHUNK_TOKENS);
    let chunks = if rough_token_count(&transcript) <= chunk_tokens {
        vec![transcript]
    } else {
        chunk_text(&transcript, chunk_tokens, 100)
    };
    info!("Extracting action items for {} in {} pass(es)", meeting_id, chunks.len());

    let client = reqwest::Client::new();
    let mut found: Vec<(&'static str, RawItem)> = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        if cancellation_token.is_some_and(|t| t.is_cancelled()) {
            return Err("Cancelled".to_string());
        }
        let completion = generate_summary(
            &client,
            llm.provider,
            llm.model_name,
            llm.api_key,
            SYSTEM_PROMPT,
            &build_prompt(chunk),
            llm.ollama_endpoint,
            llm.custom_openai_endpoint,
            llm.max_tokens,
            Some(0.1),
            llm.top_p,
            llm.app_data_dir,
            cancellation_token,
        )
        .await;
        match completion.map(|c| parse_response(&c.content)) {
            Ok(Some(parsed)) => {
                found.extend(parsed.action_items.into_iter().map(|r| ("action", r)));
                found.extend(parsed.decisions.into_iter().map(|r| ("decision", r)));
            }
            Ok(None) => warn!("Extraction pass {} returned no usable JSON", i + 1),
            Err(e) => warn!("Extraction pass {} failed: {}", i + 1, e),
        }
    }

    // Overlapping chunks often repeat an item; ground_items de-duplicates.
    let items = ground_items(meeting_id, found, &lines, "ai");
    replace_ai_items(pool, meeting_id, &items).await.map_err(|e| e.to_string())?;

    let unverified = items.iter().filter(|i| !i.evidence_verified).count();
    info!(
        "Stored {} items for {} ({} without verified evidence)",
        items.len(),
        meeting_id,
        unverified
    );
    Ok(items.len())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

const ITEM_COLUMNS: &str = "i.id, i.meeting_id, i.kind, i.title, i.description, i.owner, i.due_date, i.priority,
    i.status, i.evidence_quote, i.evidence_start_time, i.evidence_verified, i.source, i.user_edited,
    i.created_at, i.updated_at";

/// Action items and decisions for one meeting, in transcript order.
#[tauri::command]
pub async fn api_get_meeting_items(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<MeetingItem>, String> {
    let sql = format!(
        "SELECT {ITEM_COLUMNS} FROM meeting_items i WHERE i.meeting_id = ?
         ORDER BY i.kind, COALESCE(i.evidence_start_time, 1e12), i.created_at"
    );
    sqlx::query_as::<_, MeetingItem>(&sql)
        .bind(&meeting_id)
        .fetch_all(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())
}

/// Every open action item across all meetings, newest meeting first.
#[tauri::command]
pub async fn api_list_open_action_items(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<OpenActionItem>, String> {
    let sql = format!(
        "SELECT {ITEM_COLUMNS}, m.title AS meeting_title, m.created_at AS meeting_created_at
         FROM meeting_items i JOIN meetings m ON m.id = i.meeting_id
         WHERE i.kind = 'action' AND i.status = 'open'
         ORDER BY m.created_at DESC, COALESCE(i.evidence_start_time, 1e12)"
    );
    sqlx::query_as::<_, OpenActionItem>(&sql)
        .fetch_all(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())
}

/// Fields the user can change. Omitted fields are left as they are.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingItemUpdate {
    pub title: Option<String>,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub due_date: Option<String>,
    pub priority: Option<String>,
    pub status: Option<String>,
}

#[tauri::command]
pub async fn api_update_meeting_item(
    state: tauri::State<'_, AppState>,
    id: String,
    update: MeetingItemUpdate,
) -> Result<(), String> {
    if let Some(s) = &update.status {
        if !matches!(s.as_str(), "open" | "done" | "dismissed") {
            return Err(format!("Invalid status: {}", s));
        }
    }
    // A status change alone doesn't count as an edit; changing the content does.
    let content_edited = update.title.is_some()
        || update.description.is_some()
        || update.owner.is_some()
        || update.due_date.is_some()
        || update.priority.is_some();

    // Empty strings clear a field (e.g. remove a wrong owner).
    let clear = |v: Option<String>| v.map(|s| s.trim().to_string());
    sqlx::query(
        "UPDATE meeting_items SET
            title = COALESCE(NULLIF(?, ''), title),
            description = CASE WHEN ? IS NULL THEN description ELSE NULLIF(?, '') END,
            owner = CASE WHEN ? IS NULL THEN owner ELSE NULLIF(?, '') END,
            due_date = CASE WHEN ? IS NULL THEN due_date ELSE NULLIF(?, '') END,
            priority = CASE WHEN ? IS NULL THEN priority ELSE NULLIF(?, '') END,
            status = COALESCE(?, status),
            user_edited = CASE WHEN ? THEN 1 ELSE user_edited END,
            updated_at = ?
         WHERE id = ?",
    )
    .bind(clear(update.title))
    .bind(clear(update.description.clone()))
    .bind(clear(update.description))
    .bind(clear(update.owner.clone()))
    .bind(clear(update.owner))
    .bind(clear(update.due_date.clone()))
    .bind(clear(update.due_date))
    .bind(clear(update.priority.clone()))
    .bind(clear(update.priority))
    .bind(update.status)
    .bind(content_edited)
    .bind(Utc::now().to_rfc3339())
    .bind(&id)
    .execute(state.db_manager.pool())
    .await
    .map_err(|e| e.to_string())?;
    refresh_export_for_item(state.db_manager.pool(), &id).await;
    Ok(())
}

async fn refresh_export_for_item(pool: &SqlitePool, item_id: &str) {
    if let Ok(Some(meeting_id)) = sqlx::query_scalar::<_, String>("SELECT meeting_id FROM meeting_items WHERE id = ?")
        .bind(item_id)
        .fetch_optional(pool)
        .await
    {
        crate::summary::markdown_export::refresh_quietly(pool, &meeting_id).await;
    }
}

/// Adds an item the AI missed.
#[tauri::command]
pub async fn api_create_meeting_item(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    kind: String,
    title: String,
    owner: Option<String>,
    due_date: Option<String>,
) -> Result<MeetingItem, String> {
    if !matches!(kind.as_str(), "action" | "decision") {
        return Err(format!("Invalid kind: {}", kind));
    }
    if title.trim().is_empty() {
        return Err("Title is required".to_string());
    }
    let now = Utc::now().to_rfc3339();
    let item = MeetingItem {
        id: Uuid::new_v4().to_string(),
        meeting_id,
        kind,
        title: title.trim().to_string(),
        description: None,
        owner: clean_opt(owner),
        due_date: clean_opt(due_date),
        priority: None,
        status: "open".to_string(),
        evidence_quote: None,
        evidence_start_time: None,
        evidence_verified: false,
        source: "user".to_string(),
        user_edited: true,
        created_at: now.clone(),
        updated_at: now,
    };
    insert_item(state.db_manager.pool(), &item)
        .await
        .map_err(|e| e.to_string())?;
    crate::summary::markdown_export::refresh_quietly(state.db_manager.pool(), &item.meeting_id).await;
    Ok(item)
}

#[tauri::command]
pub async fn api_delete_meeting_item(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let pool = state.db_manager.pool();
    let meeting_id: Option<String> = sqlx::query_scalar("SELECT meeting_id FROM meeting_items WHERE id = ?")
        .bind(&id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM meeting_items WHERE id = ?")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(meeting_id) = meeting_id {
        crate::summary::markdown_export::refresh_quietly(pool, &meeting_id).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // Grounding, quote matching and placeholder tests live in hearken-core.
    use super::*;

    #[test]
    fn parses_json_wrapped_in_text() {
        let raw = "Here you go:\n```json\n{\"action_items\":[{\"title\":\"Send list\",\"owner\":\"Kwame\",\"due\":\"Friday\",\"evidence_quote\":\"Kwame will send the item list\"}],\"decisions\":[]}\n```";
        let parsed = parse_response(raw).unwrap();
        assert_eq!(parsed.action_items.len(), 1);
        assert_eq!(parsed.action_items[0].owner.as_deref(), Some("Kwame"));
    }
}
