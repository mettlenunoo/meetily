//! Action items and decisions, grounded in the transcript.
//!
//! Grounding rules (from the product spec):
//! - Every item carries a verbatim `evidence_quote` from the transcript.
//! - The quote is checked against the transcript; the timestamp is taken from
//!   where the quote actually appears, never from the model.
//! - Owners and due dates are left empty unless the transcript states them.
//! - Replacing AI items never touches items the user created or edited.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MeetingItem {
    pub id: String,
    pub meeting_id: String,
    pub kind: String,
    pub title: String,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub due_date: Option<String>,
    pub priority: Option<String>,
    pub status: String,
    pub evidence_quote: Option<String>,
    pub evidence_start_time: Option<f64>,
    pub evidence_verified: bool,
    pub source: String,
    pub user_edited: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// An item as a model proposes it, before grounding.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct RawItem {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub evidence_quote: Option<String>,
}

// ---------------------------------------------------------------------------
// Transcript
// ---------------------------------------------------------------------------

/// One transcript line with its position in the recording.
#[derive(Debug, Clone)]
pub struct TimedLine {
    pub start_seconds: Option<f64>,
    pub text: String,
}

pub fn format_clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{:02}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}

/// Renders lines as "[mm:ss] text", the form the model sees and quotes from.
pub fn render_transcript(lines: &[TimedLine]) -> String {
    lines
        .iter()
        .map(|l| match l.start_seconds {
            Some(s) => format!("[{}] {}", format_clock(s), l.text.trim()),
            None => l.text.trim().to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn load_timed_transcript(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<TimedLine>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, Option<f64>)>(
        "SELECT transcript, audio_start_time FROM transcripts
         WHERE meeting_id = ?
         ORDER BY COALESCE(audio_start_time, 0), timestamp",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|(t, _)| !t.trim().is_empty())
        .map(|(text, start_seconds)| TimedLine { start_seconds, text })
        .collect())
}

// ---------------------------------------------------------------------------
// Evidence grounding
// ---------------------------------------------------------------------------

pub fn normalize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Finds the line a quote came from. Accepts a quote spanning the end of one
/// line and the start of the next. Returns (verified, start_seconds).
pub fn locate_quote(lines: &[TimedLine], quote: &str) -> (bool, Option<f64>) {
    let q = normalize(quote);
    if q.split(' ').count() < 3 {
        return (false, None); // too short to verify meaningfully
    }
    let normalized: Vec<String> = lines.iter().map(|l| normalize(&l.text)).collect();
    // Prefer a single line that holds the whole quote…
    if let Some(i) = normalized.iter().position(|here| here.contains(&q)) {
        return (true, lines[i].start_seconds);
    }
    // …then a quote that runs across a line break.
    for i in 0..normalized.len().saturating_sub(1) {
        let joined = format!("{} {}", normalized[i], normalized[i + 1]);
        if joined.contains(&q) {
            return (true, lines[i].start_seconds);
        }
    }
    (false, None)
}

/// Placeholder values a model uses instead of leaving a field empty.
pub fn clean_opt(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| {
        let l = s.to_lowercase();
        !s.is_empty() && !matches!(l.as_str(), "null" | "none" | "n/a" | "unknown" | "unassigned" | "tbd")
    })
}

/// Turns proposed items into stored items: drops empty and duplicate titles, clears
/// placeholder owners/dates, and verifies each quote against the transcript (taking the
/// timestamp from where the quote really is). `kind` is "action" or "decision".
pub fn ground_items(meeting_id: &str, proposed: Vec<(&str, RawItem)>, lines: &[TimedLine], source: &str) -> Vec<MeetingItem> {
    let mut seen = std::collections::HashSet::new();
    let now = Utc::now().to_rfc3339();
    let mut items = Vec::new();
    for (kind, raw) in proposed {
        let title = raw.title.trim().to_string();
        if title.is_empty() || !seen.insert((kind.to_string(), normalize(&title))) {
            continue;
        }
        let quote = clean_opt(raw.evidence_quote);
        let (verified, start) = quote.as_deref().map(|q| locate_quote(lines, q)).unwrap_or((false, None));
        let priority = clean_opt(raw.priority)
            .map(|p| p.to_lowercase())
            .filter(|p| matches!(p.as_str(), "high" | "medium" | "low"));
        items.push(MeetingItem {
            id: Uuid::new_v4().to_string(),
            meeting_id: meeting_id.to_string(),
            kind: kind.to_string(),
            title,
            description: clean_opt(raw.description),
            owner: clean_opt(raw.owner),
            due_date: clean_opt(raw.due),
            priority,
            status: "open".to_string(),
            evidence_quote: quote,
            evidence_start_time: start,
            evidence_verified: verified,
            source: source.to_string(),
            user_edited: false,
            created_at: now.clone(),
            updated_at: now.clone(),
        });
    }
    items
}

/// Replaces a meeting's AI-generated items in one transaction. Items the user created
/// or edited are kept.
pub async fn replace_ai_items(pool: &SqlitePool, meeting_id: &str, items: &[MeetingItem]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM meeting_items WHERE meeting_id = ? AND source = 'ai' AND user_edited = 0")
        .bind(meeting_id)
        .execute(&mut *tx)
        .await?;
    for it in items {
        insert_item(&mut *tx, it).await?;
    }
    tx.commit().await
}

pub async fn insert_item<'e, E>(executor: E, it: &MeetingItem) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO meeting_items (id, meeting_id, kind, title, description, owner, due_date, priority,
            status, evidence_quote, evidence_start_time, evidence_verified, source, user_edited,
            created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&it.id)
    .bind(&it.meeting_id)
    .bind(&it.kind)
    .bind(&it.title)
    .bind(&it.description)
    .bind(&it.owner)
    .bind(&it.due_date)
    .bind(&it.priority)
    .bind(&it.status)
    .bind(&it.evidence_quote)
    .bind(it.evidence_start_time)
    .bind(it.evidence_verified)
    .bind(&it.source)
    .bind(it.user_edited)
    .bind(&it.created_at)
    .bind(&it.updated_at)
    .execute(executor)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines() -> Vec<TimedLine> {
        vec![
            TimedLine { start_seconds: Some(12.0), text: "Okay, let's start.".into() },
            TimedLine { start_seconds: Some(65.5), text: "Kwame will send the item list to Ammiel by Friday.".into() },
            TimedLine { start_seconds: Some(90.0), text: "We agreed the deployment moves to Wednesday,".into() },
            TimedLine { start_seconds: Some(95.0), text: "not Monday as planned.".into() },
        ]
    }

    #[test]
    fn locates_exact_quote_and_uses_transcript_time() {
        let (ok, t) = locate_quote(&lines(), "Kwame will send the item list to Ammiel");
        assert!(ok);
        assert_eq!(t, Some(65.5));
    }

    #[test]
    fn locates_quote_ignoring_case_and_punctuation() {
        let (ok, _) = locate_quote(&lines(), "kwame WILL send the item-list");
        assert!(ok);
    }

    #[test]
    fn locates_quote_spanning_two_lines() {
        let (ok, t) = locate_quote(&lines(), "deployment moves to Wednesday not Monday");
        assert!(ok);
        assert_eq!(t, Some(90.0));
    }

    #[test]
    fn rejects_invented_quote() {
        let (ok, t) = locate_quote(&lines(), "Desmond will fix the API tomorrow");
        assert!(!ok);
        assert_eq!(t, None);
    }

    #[test]
    fn placeholder_values_become_none() {
        assert_eq!(clean_opt(Some("null".into())), None);
        assert_eq!(clean_opt(Some(" Unknown ".into())), None);
        assert_eq!(clean_opt(Some("Ama".into())), Some("Ama".into()));
    }

    #[test]
    fn renders_transcript_with_clock() {
        let text = render_transcript(&lines());
        assert!(text.starts_with("[00:12] Okay"));
        assert_eq!(format_clock(3725.0), "01:02:05");
    }

    #[test]
    fn grounding_dedupes_cleans_and_verifies() {
        let raw = |title: &str, owner: Option<&str>, quote: &str| RawItem {
            title: title.into(),
            owner: owner.map(Into::into),
            due: Some("TBD".into()),
            evidence_quote: Some(quote.into()),
            ..Default::default()
        };
        let items = ground_items(
            "m1",
            vec![
                ("action", raw("Send item list", Some("Kwame"), "Kwame will send the item list")),
                ("action", raw("send item list ", None, "Kwame will send the item list")),
                ("decision", raw("Deploy Wednesday", Some("n/a"), "Desmond will fix the API tomorrow")),
            ],
            &lines(),
            "ai",
        );
        assert_eq!(items.len(), 2, "duplicate title dropped");
        assert_eq!(items[0].owner.as_deref(), Some("Kwame"));
        assert_eq!(items[0].due_date, None, "placeholder due cleared");
        assert!(items[0].evidence_verified);
        assert_eq!(items[0].evidence_start_time, Some(65.5));
        assert_eq!(items[1].owner, None);
        assert!(!items[1].evidence_verified);
        assert_eq!(items[1].evidence_start_time, None, "no time for an unverified quote");
    }
}
