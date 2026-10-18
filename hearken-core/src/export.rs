//! Exports a meeting as one Markdown file with YAML frontmatter:
//! summary, action items, decisions, the user's notes and the timestamped
//! transcript. Written next to the recording as `meeting.md` so the meeting
//! folder is readable without the app (Obsidian, grep, Claude, etc.).

use crate::items::format_clock;
use sqlx::SqlitePool;
use std::path::PathBuf;

pub const EXPORT_FILE_NAME: &str = "meeting.md";

struct ItemRow {
    kind: String,
    title: String,
    owner: Option<String>,
    due_date: Option<String>,
    due_at: Option<String>,
    status: String,
    evidence_quote: Option<String>,
    evidence_start_time: Option<f64>,
    evidence_verified: bool,
}

pub fn yaml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " "))
}

/// The summary is stored as JSON; the display text lives in `markdown`.
pub fn summary_markdown(result: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(result).ok()?;
    value
        .get("markdown")
        .and_then(|m| m.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub async fn build_markdown(pool: &SqlitePool, meeting_id: &str) -> Result<(String, Option<String>), String> {
    let meeting = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT title, created_at, folder_path FROM meetings WHERE id = ?",
    )
    .bind(meeting_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("Meeting not found: {}", meeting_id))?;
    let (title, created_at, folder_path) = meeting;

    let summary: Option<String> = sqlx::query_scalar("SELECT result FROM summary_processes WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .flatten();

    let notes: Option<String> = sqlx::query_scalar("SELECT notes_markdown FROM meeting_notes WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .flatten();

    let items: Vec<ItemRow> = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, Option<String>, String, Option<String>, Option<f64>, bool)>(
        "SELECT kind, title, owner, due_date, due_at, status, evidence_quote, evidence_start_time, evidence_verified
         FROM meeting_items WHERE meeting_id = ? AND status != 'dismissed'
         ORDER BY kind, COALESCE(evidence_start_time, 1e12), created_at",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?
    .into_iter()
    .map(|(kind, title, owner, due_date, due_at, status, evidence_quote, evidence_start_time, evidence_verified)| ItemRow {
        kind, title, owner, due_date, due_at, status, evidence_quote, evidence_start_time, evidence_verified,
    })
    .collect();

    let transcript: Vec<(String, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT transcript, audio_start_time, audio_end_time FROM transcripts
         WHERE meeting_id = ? ORDER BY COALESCE(audio_start_time, 0), timestamp",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let duration = transcript.iter().filter_map(|(_, _, end)| *end).fold(0.0_f64, f64::max);
    let open_actions = items.iter().filter(|i| i.kind == "action" && i.status == "open").count();

    let mut md = String::new();
    md.push_str("---\n");
    md.push_str(&format!("title: {}\n", yaml_string(&title)));
    md.push_str(&format!("date: {}\n", yaml_string(&created_at)));
    md.push_str(&format!("meeting_id: {}\n", yaml_string(meeting_id)));
    if duration > 0.0 {
        md.push_str(&format!("duration: {}\n", yaml_string(&format_clock(duration))));
    }
    md.push_str(&format!("open_action_items: {}\n", open_actions));
    md.push_str("source: hearken\n");
    md.push_str("---\n\n");
    md.push_str(&format!("# {}\n\n", title));

    if let Some(s) = summary.as_deref().and_then(summary_markdown) {
        md.push_str("## Summary\n\n");
        md.push_str(&s);
        md.push_str("\n\n");
    }

    let render_item = |it: &ItemRow, checkbox: bool| -> String {
        let mut line = if checkbox {
            format!("- [{}] {}", if it.status == "done" { "x" } else { " " }, it.title)
        } else {
            format!("- {}", it.title)
        };
        let mut meta = Vec::new();
        if let Some(o) = &it.owner { meta.push(format!("owner: {}", o)); }
        match (&it.due_date, &it.due_at) {
            (Some(spoken), Some(date)) if spoken != date => meta.push(format!("due: {} ({})", spoken, date)),
            (_, Some(date)) => meta.push(format!("due: {}", date)),
            (Some(spoken), None) => meta.push(format!("due: {}", spoken)),
            (None, None) => {}
        }
        if !meta.is_empty() { line.push_str(&format!(" ({})", meta.join(", "))); }
        if let Some(q) = &it.evidence_quote {
            let at = it.evidence_start_time.map(|t| format!("[{}] ", format_clock(t))).unwrap_or_default();
            let flag = if it.evidence_verified { "" } else { " ⚠️ not found in transcript" };
            line.push_str(&format!("\n  > {}“{}”{}", at, q, flag));
        }
        line.push('\n');
        line
    };

    let actions: Vec<&ItemRow> = items.iter().filter(|i| i.kind == "action").collect();
    if !actions.is_empty() {
        md.push_str("## Action items\n\n");
        for it in actions { md.push_str(&render_item(it, true)); }
        md.push('\n');
    }
    let decisions: Vec<&ItemRow> = items.iter().filter(|i| i.kind == "decision").collect();
    if !decisions.is_empty() {
        md.push_str("## Decisions\n\n");
        for it in decisions { md.push_str(&render_item(it, false)); }
        md.push('\n');
    }

    if let Some(n) = notes.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        md.push_str("## My notes\n\n");
        md.push_str(n);
        md.push_str("\n\n");
    }

    if !transcript.is_empty() {
        md.push_str("## Transcript\n\n");
        for (text, start, _) in &transcript {
            let text = text.trim();
            if text.is_empty() { continue; }
            match start {
                Some(s) => md.push_str(&format!("**[{}]** {}\n\n", format_clock(*s), text)),
                None => md.push_str(&format!("{}\n\n", text)),
            }
        }
    }

    Ok((md, folder_path))
}

/// Writes `meeting.md` into the meeting's recording folder (atomic replace).
/// Returns the path, or None when the meeting has no folder.
pub async fn write_to_meeting_folder(pool: &SqlitePool, meeting_id: &str) -> Result<Option<PathBuf>, String> {
    let (md, folder) = build_markdown(pool, meeting_id).await?;
    let Some(folder) = folder.map(PathBuf::from).filter(|f| f.is_dir()) else {
        return Ok(None);
    };
    let path = folder.join(EXPORT_FILE_NAME);
    let tmp = folder.join(format!(".{}.tmp", EXPORT_FILE_NAME));
    std::fs::write(&tmp, md).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_summary_markdown() {
        assert_eq!(summary_markdown(r###"{"markdown":"## Key points\n- A"}"###).as_deref(), Some("## Key points\n- A"));
        assert_eq!(summary_markdown("not json"), None);
        assert_eq!(summary_markdown(r#"{"markdown":"  "}"#), None);
    }

    #[test]
    fn yaml_strings_are_escaped() {
        assert_eq!(yaml_string("Say \"hi\"\nnow"), "\"Say \\\"hi\\\" now\"");
    }
}
