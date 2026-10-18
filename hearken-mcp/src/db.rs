//! Read-only queries over Hearken's SQLite database, formatted as Markdown for an LLM to read.

use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::str::FromStr;

/// Transcript text returned by one get_meeting call, so a long meeting can't flood the context.
const MAX_TRANSCRIPT_CHARS: usize = 60_000;

pub fn database_path() -> PathBuf {
    if let Ok(p) = std::env::var("HEARKEN_DB") {
        return PathBuf::from(p);
    }
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--db" {
            if let Some(p) = args.next() {
                return PathBuf::from(p);
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library/Application Support/com.mettlenunoo.hearken/meeting_minutes.sqlite")
}

pub async fn open_read_only(path: &PathBuf) -> anyhow::Result<SqlitePool> {
    if !path.exists() {
        anyhow::bail!("Hearken database not found at {} (open Hearken once first)", path.display());
    }
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .read_only(true)
        .create_if_missing(false);
    Ok(SqlitePoolOptions::new().max_connections(2).connect_with(opts).await?)
}

// ---------- formatting helpers ----------

pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

/// "2026-10-18T15:01:04.123Z" -> "2026-10-18 15:01"
fn short_date(ts: &str) -> String {
    ts.get(..16).unwrap_or(ts).replace('T', " ")
}

/// Same FTS5 query shape as the app's sidebar search: every word required, prefix match on the last.
pub fn build_fts_query(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| t.chars().any(|c| c.is_alphanumeric()))
        .collect();
    let (last, rest) = terms.split_last()?;
    let mut parts: Vec<String> = rest.iter().map(|t| format!("\"{t}\"")).collect();
    parts.push(format!("\"{last}\"*"));
    Some(parts.join(" AND "))
}

/// Summaries are stored as JSON with the display text in `markdown`.
fn summary_markdown(result: &str) -> Option<String> {
    let value: Value = serde_json::from_str(result).ok()?;
    value.get("markdown").and_then(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn clean_summary_snippet(snippet: &str) -> String {
    let cleaned: String = snippet
        .replace("\\n", " ")
        .chars()
        .filter(|c| !matches!(c, '{' | '}' | '[' | ']' | '"' | '\\'))
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn arg_limit(args: &Value, default: i64, max: i64) -> i64 {
    args.get("limit").and_then(Value::as_i64).unwrap_or(default).clamp(1, max)
}

fn write_item(out: &mut String, row: &sqlx::sqlite::SqliteRow, with_meeting: bool) {
    let kind: String = row.get("kind");
    let title: String = row.get("title");
    let status: String = row.get("status");
    let owner: Option<String> = row.get("owner");
    let due: Option<String> = row.get("due_date");
    let quote: Option<String> = row.get("evidence_quote");
    let at: Option<f64> = row.get("evidence_start_time");
    let verified: i64 = row.get("evidence_verified");
    let source: String = row.get("source");

    let check = match (kind.as_str(), status.as_str()) {
        ("decision", _) => "-",
        (_, "done") => "- [x]",
        (_, "dismissed") => "- [~] (dismissed)",
        _ => "- [ ]",
    };
    let _ = write!(out, "{check} {title}");
    let mut meta = Vec::new();
    meta.push(format!("owner: {}", owner.as_deref().unwrap_or("not stated")));
    if kind == "action" {
        meta.push(format!("due: {}", due.as_deref().unwrap_or("not stated")));
    }
    if source == "user" {
        meta.push("added by user".into());
    }
    let _ = write!(out, " ({})", meta.join(", "));
    if with_meeting {
        let mt: String = row.get("meeting_title");
        let md: String = row.get("meeting_created_at");
        let mid: String = row.get("meeting_id");
        let _ = write!(out, " — meeting \"{mt}\", {} [id {mid}]", short_date(&md));
    }
    out.push('\n');
    if let Some(q) = quote.filter(|q| !q.trim().is_empty()) {
        let when = at.map(|t| format!("[{}] ", clock(t))).unwrap_or_default();
        let flag = if verified == 1 { "" } else { " (UNVERIFIED: quote not found in transcript)" };
        let _ = writeln!(out, "  > {when}\"{}\"{flag}", q.trim());
    }
}

// ---------- tools ----------

pub async fn search_meetings(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    let query = arg_str(args, "query").ok_or("query is required")?;
    let fts = build_fts_query(query).ok_or("query has no searchable words")?;
    let limit = arg_limit(args, 20, 100);

    let rows = sqlx::query(
        "SELECT s.meeting_id, m.title, m.created_at, s.source,
                snippet(search_index, 0, '**', '**', '…', 16) AS snip,
                t.audio_start_time
         FROM search_index s
         JOIN meetings m ON m.id = s.meeting_id
         LEFT JOIN transcripts t ON s.source = 'transcript' AND t.id = s.ref_id
         WHERE search_index MATCH ?1
         ORDER BY bm25(search_index)
         LIMIT ?2",
    )
    .bind(&fts)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("search failed: {e}"))?;

    if rows.is_empty() {
        return Ok(format!("No matches for \"{query}\" in any meeting."));
    }
    let mut out = format!("{} match(es) for \"{query}\":\n\n", rows.len());
    for r in rows {
        let source: String = r.get("source");
        let mut snip: String = r.get("snip");
        if source == "summary" {
            snip = clean_summary_snippet(&snip);
        }
        let at: Option<f64> = r.get("audio_start_time");
        let when = at.map(|t| format!(" at [{}]", clock(t))).unwrap_or_default();
        let title: String = r.get("title");
        let created: String = r.get("created_at");
        let mid: String = r.get("meeting_id");
        let _ = writeln!(
            out,
            "- \"{title}\" ({}) — {source}{when} [id {mid}]\n  {}",
            short_date(&created),
            snip.replace('\n', " ")
        );
    }
    Ok(out)
}

pub async fn list_meetings(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    let limit = arg_limit(args, 20, 200);
    let title = arg_str(args, "title_contains").map(|t| format!("%{}%", t.to_lowercase()));
    let since = arg_str(args, "since");

    let rows = sqlx::query(
        "SELECT m.id, m.title, m.created_at,
                (SELECT MAX(audio_end_time) FROM transcripts t WHERE t.meeting_id = m.id) AS length_s,
                EXISTS(SELECT 1 FROM summary_processes p WHERE p.meeting_id = m.id
                       AND p.result IS NOT NULL AND p.result != '') AS has_summary,
                (SELECT COUNT(*) FROM meeting_items i WHERE i.meeting_id = m.id
                       AND i.kind = 'action' AND i.status = 'open') AS open_actions
         FROM meetings m
         WHERE (?1 IS NULL OR lower(m.title) LIKE ?1)
           AND (?2 IS NULL OR substr(m.created_at, 1, 10) >= ?2)
         ORDER BY m.created_at DESC
         LIMIT ?3",
    )
    .bind(title)
    .bind(since)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("listing meetings failed: {e}"))?;

    if rows.is_empty() {
        return Ok("No meetings found.".into());
    }
    let mut out = String::new();
    for r in rows {
        let id: String = r.get("id");
        let title: String = r.get("title");
        let created: String = r.get("created_at");
        let length: Option<f64> = r.get("length_s");
        let has_summary: bool = r.get("has_summary");
        let open: i64 = r.get("open_actions");
        let _ = writeln!(
            out,
            "- \"{title}\" — {}, {}, {}, {open} open action item(s) [id {id}]",
            short_date(&created),
            length.map(clock).unwrap_or_else(|| "no transcript".into()),
            if has_summary { "summarised" } else { "no summary" },
        );
    }
    Ok(out)
}

pub async fn get_meeting(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    let id = arg_str(args, "meeting_id").ok_or("meeting_id is required")?;
    let meeting = sqlx::query("SELECT title, created_at, folder_path FROM meetings WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("No meeting with id {id}"))?;
    let title: String = meeting.get("title");
    let created: String = meeting.get("created_at");

    let mut out = format!("# {title}\n\nDate: {}  \nMeeting id: {id}\n", short_date(&created));

    let summary: Option<String> =
        sqlx::query_scalar("SELECT result FROM summary_processes WHERE meeting_id = ?1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
            .flatten();
    out.push_str("\n## Summary\n\n");
    out.push_str(&summary.as_deref().and_then(summary_markdown).unwrap_or_else(|| "(no summary yet)".into()));
    out.push('\n');

    let items = sqlx::query(
        "SELECT * FROM meeting_items WHERE meeting_id = ?1 AND status != 'dismissed'
         ORDER BY kind, COALESCE(evidence_start_time, 1e12), created_at",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    for (kind, heading) in [("action", "Action items"), ("decision", "Decisions")] {
        let mut section = String::new();
        for r in items.iter().filter(|r| r.get::<String, _>("kind") == kind) {
            write_item(&mut section, r, false);
        }
        if !section.is_empty() {
            let _ = write!(out, "\n## {heading}\n\n{section}");
        }
    }

    let notes: Option<String> =
        sqlx::query_scalar("SELECT notes_markdown FROM meeting_notes WHERE meeting_id = ?1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
            .flatten();
    if let Some(n) = notes.filter(|n| !n.trim().is_empty()) {
        let _ = write!(out, "\n## Notes\n\n{}\n", n.trim());
    }

    if args.get("include_transcript").and_then(Value::as_bool).unwrap_or(false) {
        let from = args.get("from_seconds").and_then(Value::as_f64);
        let to = args.get("to_seconds").and_then(Value::as_f64);
        let rows = sqlx::query(
            "SELECT transcript, audio_start_time, speaker FROM transcripts
             WHERE meeting_id = ?1
               AND (?2 IS NULL OR COALESCE(audio_start_time, 0) >= ?2)
               AND (?3 IS NULL OR COALESCE(audio_start_time, 0) <= ?3)
             ORDER BY COALESCE(audio_start_time, 0), timestamp",
        )
        .bind(id)
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

        out.push_str("\n## Transcript\n\n");
        if rows.is_empty() {
            out.push_str("(no transcript in this range)\n");
        }
        let mut used = 0usize;
        for r in rows {
            let text: String = r.get("transcript");
            let at: Option<f64> = r.get("audio_start_time");
            let speaker: Option<String> = r.get("speaker");
            let line = format!(
                "[{}]{} {}\n",
                at.map(clock).unwrap_or_else(|| "--:--".into()),
                speaker.map(|s| format!(" {s}:")).unwrap_or_default(),
                text.trim()
            );
            if used + line.len() > MAX_TRANSCRIPT_CHARS {
                let _ = writeln!(
                    out,
                    "\n(transcript truncated; call get_meeting again with from_seconds={} for the rest)",
                    at.map(|t| t.floor() as i64).unwrap_or(0)
                );
                break;
            }
            used += line.len();
            out.push_str(&line);
        }
    }
    Ok(out)
}

pub async fn list_action_items(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    let status = arg_str(args, "status").unwrap_or("open");
    let kind = arg_str(args, "kind").unwrap_or("action");
    let owner = arg_str(args, "owner_contains").map(|o| format!("%{}%", o.to_lowercase()));
    let meeting = arg_str(args, "meeting_id");
    let limit = arg_limit(args, 100, 500);

    let rows = sqlx::query(
        "SELECT i.*, m.title AS meeting_title, m.created_at AS meeting_created_at
         FROM meeting_items i JOIN meetings m ON m.id = i.meeting_id
         WHERE (?1 = 'any' OR i.status = ?1)
           AND (?2 = 'any' OR i.kind = ?2)
           AND (?3 IS NULL OR lower(COALESCE(i.owner, '')) LIKE ?3)
           AND (?4 IS NULL OR i.meeting_id = ?4)
         ORDER BY m.created_at DESC, COALESCE(i.evidence_start_time, 1e12)
         LIMIT ?5",
    )
    .bind(status)
    .bind(kind)
    .bind(owner)
    .bind(meeting)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("listing items failed: {e}"))?;

    if rows.is_empty() {
        return Ok(format!("No {kind} items with status {status}."));
    }
    let mut out = format!("{} item(s):\n\n", rows.len());
    for r in &rows {
        write_item(&mut out, r, true);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_formats() {
        assert_eq!(clock(5.4), "00:05");
        assert_eq!(clock(754.0), "12:34");
        assert_eq!(clock(3725.0), "1:02:05");
    }

    #[test]
    fn fts_query_quotes_terms_and_prefixes_last() {
        assert_eq!(build_fts_query("budget rev").unwrap(), "\"budget\" AND \"rev\"*");
        assert_eq!(build_fts_query("say \"hi\"").unwrap(), "\"say\" AND \"hi\"*");
        assert!(build_fts_query("  -- ").is_none());
    }

    #[test]
    fn summary_markdown_reads_json() {
        assert_eq!(summary_markdown(r##"{"markdown":"# Hi\n"}"##).as_deref(), Some("# Hi"));
        assert!(summary_markdown("not json").is_none());
    }
}
