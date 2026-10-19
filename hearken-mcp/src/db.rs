//! Queries over Hearken's SQLite database, formatted as Markdown for an LLM to read, plus the
//! two narrow writes Claude may make: a meeting's summary and its AI action items/decisions.

use hearken_core::items::{ground_items, load_timed_transcript, meeting_day, replace_ai_items, RawItem};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

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

/// Opens the existing database (never creates one). Writes are limited to the save_* functions
/// below; nothing here deletes meetings or touches transcripts, notes or audio.
pub async fn open(path: &PathBuf) -> anyhow::Result<SqlitePool> {
    if !path.exists() {
        anyhow::bail!("Hearken database not found at {} (open Hearken once first)", path.display());
    }
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(10)); // wait if Hearken is mid-write
    Ok(SqlitePoolOptions::new().max_connections(2).connect_with(opts).await?)
}

// ---------- formatting helpers ----------

pub use hearken_core::items::format_clock as clock;

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

use hearken_core::export::summary_markdown;

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

async fn has_table(pool: &SqlitePool, name: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1")
        .bind(name)
        .fetch_one(pool)
        .await
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// True once Hearken has created the projects tables (older databases don't have them).
async fn has_projects(pool: &SqlitePool) -> bool {
    has_table(pool, "projects").await
}

/// "## Documents" lines for a meeting or project, or "" when there are none.
async fn document_list(pool: &SqlitePool, column: &str, id: &str) -> Result<String, String> {
    if !has_table(pool, "documents").await {
        return Ok(String::new());
    }
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(&format!(
        "SELECT id, title, generated_by, updated_at FROM documents WHERE {column} = ?1 AND status = 'ready' ORDER BY updated_at DESC"
    ))
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok(String::new());
    }
    let mut out = String::from("\n## Documents (read with get_document)\n\n");
    for (doc_id, title, by, updated) in rows {
        let _ = writeln!(out, "- {title} — by {by}, {} [document {doc_id}]", short_date(&updated));
    }
    Ok(out)
}

/// SQL condition that is true when the meeting in `col` may be shown to Claude: it isn't
/// linked (as main or extra) to any project marked local-only.
async fn visible(pool: &SqlitePool, col: &str) -> String {
    if has_projects(pool).await {
        format!(
            "NOT EXISTS (SELECT 1 FROM meeting_projects vmp JOIN projects vp ON vp.id = vmp.project_id \
             WHERE vmp.meeting_id = {col} AND vp.privacy = 'local')"
        )
    } else {
        "1".into()
    }
}

fn today() -> String {
    chrono::Local::now().date_naive().format("%Y-%m-%d").to_string()
}

fn write_item(out: &mut String, row: &sqlx::sqlite::SqliteRow, with_meeting: bool) {
    let kind: String = row.get("kind");
    let title: String = row.get("title");
    let status: String = row.get("status");
    let owner: Option<String> = row.get("owner");
    let due: Option<String> = row.get("due_date");
    let due_at: Option<String> = row.try_get("due_at").ok().flatten();
    let id: String = row.get("id");
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
        meta.push(match (due.as_deref(), due_at.as_deref()) {
            (Some(words), Some(date)) if words != date => format!("due: {words} = {date}"),
            (_, Some(date)) => format!("due: {date}"),
            (Some(words), None) => format!("due: {words} (no exact date)"),
            (None, None) => "due: not stated".into(),
        });
        if status == "open" && due_at.as_deref().is_some_and(|d| d < today().as_str()) {
            meta.push("OVERDUE".into());
        }
    }
    if source == "user" {
        meta.push("added by user".into());
    }
    let _ = write!(out, " ({}) [item {id}]", meta.join(", "));
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

    let sql = format!(
        "SELECT s.meeting_id, m.title, m.created_at, s.source,
                snippet(search_index, 0, '**', '**', '…', 16) AS snip,
                t.audio_start_time
         FROM search_index s
         JOIN meetings m ON m.id = s.meeting_id
         LEFT JOIN transcripts t ON s.source = 'transcript' AND t.id = s.ref_id
         WHERE search_index MATCH ?1 AND {}
         ORDER BY bm25(search_index)
         LIMIT ?2",
        visible(pool, "s.meeting_id").await
    );
    let rows = sqlx::query(&sql)
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

    let project_col = if has_projects(pool).await {
        "(SELECT p.name FROM meeting_projects mp JOIN projects p ON p.id = mp.project_id
          WHERE mp.meeting_id = m.id AND mp.is_primary = 1)"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT m.id, m.title, m.created_at, {project_col} AS project,
                (SELECT MAX(audio_end_time) FROM transcripts t WHERE t.meeting_id = m.id) AS length_s,
                EXISTS(SELECT 1 FROM summary_processes p WHERE p.meeting_id = m.id
                       AND p.result IS NOT NULL AND p.result != '') AS has_summary,
                (SELECT COUNT(*) FROM meeting_items i WHERE i.meeting_id = m.id
                       AND i.kind = 'action' AND i.status = 'open') AS open_actions
         FROM meetings m
         WHERE (?1 IS NULL OR lower(m.title) LIKE ?1)
           AND (?2 IS NULL OR substr(m.created_at, 1, 10) >= ?2)
           AND {}
         ORDER BY m.created_at DESC
         LIMIT ?3",
        visible(pool, "m.id").await
    );
    let rows = sqlx::query(&sql)
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
        let project: Option<String> = r.get("project");
        let project = project.map(|p| format!(", project: {p}")).unwrap_or_default();
        let _ = writeln!(
            out,
            "- \"{title}\" — {}, {}, {}, {open} open action item(s){project} [id {id}]",
            short_date(&created),
            length.map(clock).unwrap_or_else(|| "no transcript".into()),
            if has_summary { "summarised" } else { "no summary" },
        );
    }
    Ok(out)
}

pub async fn get_meeting(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    let id = arg_str(args, "meeting_id").ok_or("meeting_id is required")?;
    require_meeting(pool, id).await?;
    let meeting = sqlx::query("SELECT title, created_at, folder_path FROM meetings WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("No meeting with id {id}"))?;
    let title: String = meeting.get("title");
    let created: String = meeting.get("created_at");

    let mut out = format!("# {title}\n\nDate: {}  \nMeeting id: {id}\n", short_date(&created));
    if has_projects(pool).await {
        let projects: Vec<(String, bool)> = sqlx::query_as(
            "SELECT p.name, mp.is_primary = 1 FROM meeting_projects mp JOIN projects p ON p.id = mp.project_id
             WHERE mp.meeting_id = ?1 ORDER BY mp.is_primary DESC, p.name",
        )
        .bind(id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        if !projects.is_empty() {
            let names: Vec<String> = projects
                .iter()
                .map(|(n, main)| if *main { format!("{n} (main)") } else { n.clone() })
                .collect();
            let _ = writeln!(out, "Projects: {}", names.join(", "));
        }
    }

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
    out.push_str(&document_list(pool, "meeting_id", id).await?);

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
    let due = arg_str(args, "due").unwrap_or("any");
    let projects = has_projects(pool).await;
    let project_id = match arg_str(args, "project") {
        Some(p) if projects => Some(find_project(pool, p).await?.0),
        Some(_) => return Err("this Hearken database has no projects yet".into()),
        None => None,
    };
    let due_col = if projects { "i.due_at" } else { "NULL" };
    let project_cond = if projects {
        "(?6 IS NULL OR EXISTS (SELECT 1 FROM meeting_projects fp WHERE fp.meeting_id = i.meeting_id AND fp.project_id = ?6))"
    } else {
        "?6 IS NULL"
    };
    let week = (chrono::Local::now().date_naive() + chrono::Duration::days(7)).format("%Y-%m-%d").to_string();
    let sql = format!(
        "SELECT i.*, m.title AS meeting_title, m.created_at AS meeting_created_at
         FROM meeting_items i JOIN meetings m ON m.id = i.meeting_id
         WHERE (?1 = 'any' OR i.status = ?1)
           AND (?2 = 'any' OR i.kind = ?2)
           AND (?3 IS NULL OR lower(COALESCE(i.owner, '')) LIKE ?3)
           AND (?4 IS NULL OR i.meeting_id = ?4)
           AND {project_cond}
           AND (CASE ?7 WHEN 'overdue' THEN {due_col} < ?8
                        WHEN 'week' THEN {due_col} <= ?9
                        WHEN 'undated' THEN {due_col} IS NULL
                        ELSE 1 END)
           AND {}
         ORDER BY {due_col} IS NULL, {due_col}, m.created_at DESC, COALESCE(i.evidence_start_time, 1e12)
         LIMIT ?5",
        visible(pool, "i.meeting_id").await
    );
    let rows = sqlx::query(&sql)
    .bind(status)
    .bind(kind)
    .bind(owner)
    .bind(meeting)
    .bind(limit)
    .bind(project_id)
    .bind(due)
    .bind(today())
    .bind(week)
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

// ---------- writes ----------

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// The meeting's title, if it exists and isn't in a local-only project (those are reported
/// as not found, so Claude can't learn they exist).
async fn require_meeting(pool: &SqlitePool, id: &str) -> Result<String, String> {
    let sql = format!("SELECT title FROM meetings m WHERE m.id = ?1 AND {}", visible(pool, "m.id").await);
    sqlx::query_scalar::<_, String>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("No meeting with id {id}"))
}

/// Mirrors the app: the meeting title is shown separately, so a leading `# Title` is dropped.
fn summary_body(markdown: &str) -> Result<String, String> {
    let lower = markdown.to_lowercase();
    if lower.contains("<think") || lower.contains("</think") {
        return Err("summary must not contain <thinking> tags".into());
    }
    let trimmed = markdown.trim();
    let body = if trimmed.starts_with("# ") {
        trimmed.split_once('\n').map(|(_, rest)| rest.trim_start()).unwrap_or("")
    } else {
        trimmed
    };
    if body.trim().is_empty() {
        return Err("summary is empty (a lone `# Title` line doesn't count)".into());
    }
    Ok(body.trim_end().to_string())
}

/// Best-effort refresh of meeting.md, like the app does after a summary or item change.
async fn refresh_export(pool: &SqlitePool, id: &str) -> String {
    match hearken_core::export::write_to_meeting_folder(pool, id).await {
        Ok(Some(p)) => format!("meeting.md updated at {}", p.display()),
        Ok(None) => "no meeting folder, so meeting.md was not written".into(),
        Err(e) => format!("meeting.md not updated: {e}"),
    }
}

pub async fn save_summary(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    let id = arg_str(args, "meeting_id").ok_or("meeting_id is required")?;
    let markdown = args.get("markdown").and_then(Value::as_str).ok_or("markdown is required")?;
    let title = require_meeting(pool, id).await?;
    let body = summary_body(markdown)?;

    let status: Option<String> = sqlx::query_scalar("SELECT status FROM summary_processes WHERE meeting_id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    if status.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("pending") || s.eq_ignore_ascii_case("processing")) {
        return Err("Hearken is generating a summary for this meeting right now; try again when it has finished".into());
    }

    let result = json!({"markdown": body, "generated_by": "claude-via-mcp", "reasoning_stripped": false}).to_string();
    let ts = now();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    if status.is_some() {
        // Keep the previous summary in the backup columns the app already has.
        sqlx::query(
            "UPDATE summary_processes
             SET result_backup = CASE WHEN result IS NOT NULL AND result != '' THEN result ELSE result_backup END,
                 result_backup_timestamp = CASE WHEN result IS NOT NULL AND result != '' THEN ?1 ELSE result_backup_timestamp END,
                 result = ?2, status = 'completed', error = NULL, updated_at = ?1, end_time = ?1
             WHERE meeting_id = ?3",
        )
        .bind(&ts)
        .bind(&result)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    } else {
        sqlx::query(
            "INSERT INTO summary_processes
                (meeting_id, status, created_at, updated_at, start_time, end_time, result, chunk_count, processing_time)
             VALUES (?1, 'completed', ?2, ?2, ?2, ?2, ?3, 0, 0.0)",
        )
        .bind(id)
        .bind(&ts)
        .bind(&result)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }
    sqlx::query("UPDATE meetings SET updated_at = ?1 WHERE id = ?2")
        .bind(&ts)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;

    let export = refresh_export(pool, id).await;
    let backed_up = if status.is_some() { " The previous summary was kept as a backup." } else { "" };
    Ok(format!(
        "Saved the summary for \"{title}\" ({} characters).{backed_up} {export}. It shows in Hearken next time the meeting is opened.",
        body.chars().count()
    ))
}

#[derive(serde::Deserialize, Default)]
struct ItemsArgs {
    #[serde(default)]
    action_items: Vec<RawItem>,
    #[serde(default)]
    decisions: Vec<RawItem>,
}

pub async fn save_meeting_items(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    let id = arg_str(args, "meeting_id").ok_or("meeting_id is required")?;
    let title = require_meeting(pool, id).await?;
    let parsed: ItemsArgs = serde_json::from_value(args.clone()).map_err(|e| format!("invalid items: {e}"))?;

    let lines = load_timed_transcript(pool, id).await.map_err(|e| e.to_string())?;
    if lines.is_empty() {
        return Err("this meeting has no transcript, so items can't be checked against it".into());
    }

    let proposed: Vec<(&str, RawItem)> = parsed
        .action_items
        .into_iter()
        .map(|r| ("action", r))
        .chain(parsed.decisions.into_iter().map(|r| ("decision", r)))
        .collect();
    let items = ground_items(id, proposed, &lines, "ai", meeting_day(pool, id).await);
    replace_ai_items(pool, id, &items).await.map_err(|e| e.to_string())?;

    let kept: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM meeting_items WHERE meeting_id = ?1 AND (source = 'user' OR user_edited = 1)",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    let export = refresh_export(pool, id).await;

    let actions = items.iter().filter(|i| i.kind == "action").count();
    let decisions = items.len() - actions;
    let mut out = format!(
        "Saved {actions} action item(s) and {decisions} decision(s) for \"{title}\", replacing the previous AI items. \
         {kept} item(s) you created or edited in Hearken were left untouched. {export}.\n"
    );
    let unverified: Vec<_> = items.iter().filter(|i| !i.evidence_verified).collect();
    if unverified.is_empty() {
        out.push_str("\nEvery quote was found in the transcript; times come from where each quote appears.");
    } else {
        let _ = writeln!(
            out,
            "\n{} item(s) are marked UNVERIFIED because their quote was not found word-for-word in the transcript \
             (needs at least 3 consecutive words, punctuation and case ignored):",
            unverified.len()
        );
        for i in unverified {
            let _ = writeln!(out, "- {} — quote: {:?}", i.title, i.evidence_quote.as_deref().unwrap_or("(none)"));
        }
        out.push_str(
            "\nTo fix them, call save_meeting_items again with the full list, copying each quote exactly from one or two \
             adjacent transcript lines (without the [mm:ss] prefix).",
        );
    }
    Ok(out)
}

// ---------- projects and tracking ----------

/// A project by id or (case-insensitive) name, unless it is local-only. Returns (id, name).
async fn find_project(pool: &SqlitePool, key: &str) -> Result<(String, String), String> {
    if !has_projects(pool).await {
        return Err("this Hearken database has no projects yet (open the latest Hearken once)".into());
    }
    sqlx::query_as::<_, (String, String)>(
        "SELECT id, name FROM projects WHERE (id = ?1 OR name = ?1 COLLATE NOCASE) AND privacy != 'local'",
    )
    .bind(key.trim())
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("No project called {key:?} (call list_projects for the names)"))
}

pub async fn list_projects(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    if !has_projects(pool).await {
        return Ok("No projects yet.".into());
    }
    let archived = args.get("include_archived").and_then(Value::as_bool).unwrap_or(false);
    let sql = format!(
        "SELECT p.id, p.name, p.kind, p.status, p.description,
            (SELECT COUNT(*) FROM meeting_projects mp WHERE mp.project_id = p.id AND {vis}) AS meetings,
            (SELECT COUNT(*) FROM meeting_items i JOIN meeting_projects mp ON mp.meeting_id = i.meeting_id
              WHERE mp.project_id = p.id AND i.kind = 'action' AND i.status = 'open' AND {vis}) AS open_actions,
            (SELECT COUNT(*) FROM meeting_items i JOIN meeting_projects mp ON mp.meeting_id = i.meeting_id
              WHERE mp.project_id = p.id AND i.kind = 'action' AND i.status = 'open'
                AND i.due_at IS NOT NULL AND i.due_at < ?2 AND {vis}) AS overdue
         FROM projects p
         WHERE p.privacy != 'local' AND (?1 OR p.status = 'active')
         ORDER BY p.status = 'archived', p.kind = 'course', p.name",
        vis = visible(pool, "mp.meeting_id").await
    );
    let rows = sqlx::query(&sql).bind(archived).bind(today()).fetch_all(pool).await.map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok("No projects yet.".into());
    }
    let mut out = String::new();
    for r in rows {
        let (id, name, kind, status): (String, String, String, String) = (r.get("id"), r.get("name"), r.get("kind"), r.get("status"));
        let desc: Option<String> = r.get("description");
        let (m, open, overdue): (i64, i64, i64) = (r.get("meetings"), r.get("open_actions"), r.get("overdue"));
        let _ = write!(out, "- {name} ({kind}{}) — {m} meeting(s), {open} open action(s)", if status == "archived" { ", archived" } else { "" });
        if overdue > 0 {
            let _ = write!(out, ", {overdue} overdue");
        }
        let _ = writeln!(out, " [id {id}]");
        if let Some(d) = desc.filter(|d| !d.trim().is_empty()) {
            let _ = writeln!(out, "  {}", d.trim());
        }
    }
    Ok(out)
}

pub async fn get_project(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    let key = arg_str(args, "project").ok_or("project (name or id) is required")?;
    let (id, name) = find_project(pool, key).await?;
    let p = sqlx::query("SELECT * FROM projects WHERE id = ?1").bind(&id).fetch_one(pool).await.map_err(|e| e.to_string())?;
    let mut out = format!("# {name}\n\nKind: {}  \nStatus: {}\n", p.get::<String, _>("kind"), p.get::<String, _>("status"));
    for (col, label) in [("description", "Description"), ("people", "People"), ("keywords", "Keywords"), ("slack_channels", "Slack channels")] {
        if let Some(v) = p.get::<Option<String>, _>(col).filter(|v| !v.trim().is_empty()) {
            let _ = writeln!(out, "{label}: {}", v.trim());
        }
    }

    let vis = visible(pool, "m.id").await;
    let meetings = sqlx::query(&format!(
        "SELECT m.id, m.title, m.created_at, mp.is_primary = 1 AS main
         FROM meeting_projects mp JOIN meetings m ON m.id = mp.meeting_id
         WHERE mp.project_id = ?1 AND {vis} ORDER BY m.created_at DESC"
    ))
    .bind(&id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let _ = writeln!(out, "\n## Meetings ({})\n", meetings.len());
    for r in &meetings {
        let (mid, title, created): (String, String, String) = (r.get("id"), r.get("title"), r.get("created_at"));
        let main: bool = r.get("main");
        let _ = writeln!(out, "- \"{title}\" — {}{} [id {mid}]", short_date(&created), if main { "" } else { " (also linked)" });
    }

    out.push_str(&document_list(pool, "project_id", &id).await?);

    for (kind, heading) in [("action", "Action items"), ("decision", "Decisions")] {
        let items = sqlx::query(&format!(
            "SELECT i.*, m.title AS meeting_title, m.created_at AS meeting_created_at
             FROM meeting_items i JOIN meetings m ON m.id = i.meeting_id
             JOIN meeting_projects mp ON mp.meeting_id = m.id AND mp.project_id = ?1
             WHERE i.kind = ?2 AND i.status != 'dismissed' AND {vis}
             ORDER BY i.status != 'open', i.due_at IS NULL, i.due_at, m.created_at DESC"
        ))
        .bind(&id)
        .bind(kind)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        if !items.is_empty() {
            let _ = writeln!(out, "\n## {heading}\n");
            for r in &items {
                write_item(&mut out, r, true);
            }
        }
    }
    Ok(out)
}

pub async fn assign_meeting_to_project(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    let meeting_id = arg_str(args, "meeting_id").ok_or("meeting_id is required")?;
    let title = require_meeting(pool, meeting_id).await?;
    let (pid, name) = find_project(pool, arg_str(args, "project").ok_or("project is required")?).await?;
    let role = arg_str(args, "as").unwrap_or("main");
    if !matches!(role, "main" | "extra") {
        return Err("as must be \"main\" or \"extra\"".into());
    }
    let ts = now();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    if role == "main" {
        // The previous main project stays linked, as an extra, so nothing is lost.
        sqlx::query("UPDATE meeting_projects SET is_primary = 0 WHERE meeting_id = ?1 AND is_primary = 1")
            .bind(meeting_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        sqlx::query(
            "INSERT INTO meeting_projects (meeting_id, project_id, is_primary, created_at) VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(meeting_id, project_id) DO UPDATE SET is_primary = 1",
        )
        .bind(meeting_id).bind(&pid).bind(&ts).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        sqlx::query(
            "UPDATE project_suggestions SET status = CASE WHEN project_id = ?2 THEN 'accepted' ELSE 'rejected' END
             WHERE meeting_id = ?1 AND status = 'pending'",
        )
        .bind(meeting_id).bind(&pid).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    } else {
        sqlx::query("INSERT OR IGNORE INTO meeting_projects (meeting_id, project_id, is_primary, created_at) VALUES (?1, ?2, 0, ?3)")
            .bind(meeting_id).bind(&pid).bind(&ts).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(format!("\"{title}\" is now filed under {name}{}.", if role == "main" { " (main project)" } else { " (extra project)" }))
}

pub async fn update_action_item(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    let item_id = arg_str(args, "item_id").ok_or("item_id is required")?;
    let row = sqlx::query("SELECT meeting_id, title, kind FROM meeting_items WHERE id = ?1")
        .bind(item_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("No item with id {item_id}"))?;
    let meeting_id: String = row.get("meeting_id");
    require_meeting(pool, &meeting_id).await.map_err(|_| format!("No item with id {item_id}"))?;

    let status = arg_str(args, "status");
    if let Some(st) = status {
        if !matches!(st, "open" | "done" | "dismissed") {
            return Err("status must be open, done or dismissed".into());
        }
    }
    let title = arg_str(args, "title");
    // Empty string clears; absent leaves as is.
    let owner = args.get("owner").and_then(Value::as_str).map(str::trim);
    let due_at = match args.get("due_date").and_then(Value::as_str).map(str::trim) {
        Some("") => Some(String::new()),
        Some(d) => Some(
            chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
                .map_err(|_| format!("due_date must be YYYY-MM-DD, got {d:?}"))?
                .format("%Y-%m-%d")
                .to_string(),
        ),
        None => None,
    };
    if status.is_none() && title.is_none() && owner.is_none() && due_at.is_none() {
        return Err("nothing to change: give status, title, owner or due_date".into());
    }
    if due_at.is_some() && !has_projects(pool).await {
        return Err("due dates need the latest Hearken (open it once to update the database)".into());
    }
    // Changing the content counts as the user's edit, so re-extraction won't overwrite it.
    let edited = title.is_some() || owner.is_some() || due_at.is_some();
    let due_sql = if due_at.is_some() {
        "due_at = NULLIF(?5, ''), due_date = CASE WHEN ?5 = '' THEN NULL ELSE ?5 END, reminded_on = NULL,"
    } else {
        ""
    };
    let sql = format!(
        "UPDATE meeting_items SET
            status = COALESCE(?2, status),
            title = COALESCE(?3, title),
            owner = CASE WHEN ?4 IS NULL THEN owner ELSE NULLIF(?4, '') END,
            {due_sql}
            user_edited = CASE WHEN ?6 THEN 1 ELSE user_edited END,
            updated_at = ?7
         WHERE id = ?1"
    );
    sqlx::query(&sql)
        .bind(item_id)
        .bind(status)
        .bind(title)
        .bind(owner)
        .bind(due_at.clone().unwrap_or_default())
        .bind(edited)
        .bind(now())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    let export = refresh_export(pool, &meeting_id).await;

    let updated = sqlx::query("SELECT i.*, m.title AS meeting_title, m.created_at AS meeting_created_at FROM meeting_items i JOIN meetings m ON m.id = i.meeting_id WHERE i.id = ?1")
        .bind(item_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    let mut out = String::from("Updated:\n");
    write_item(&mut out, &updated, true);
    let _ = write!(out, "{export}.");
    Ok(out)
}

// ---------- documents ----------

pub async fn get_document(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    let id = arg_str(args, "document_id").ok_or("document_id is required")?;
    if !has_table(pool, "documents").await {
        return Err(format!("No document with id {id}"));
    }
    let row: Option<(Option<String>, Option<String>, String, String, String)> = sqlx::query_as(
        "SELECT meeting_id, project_id, title, markdown, generated_by FROM documents WHERE id = ?1 AND status = 'ready'",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let (meeting_id, project_id, title, markdown, by) = row.ok_or_else(|| format!("No document with id {id}"))?;
    // Same privacy rules as everything else: hidden sources are reported as not found.
    let visible_source = match (&meeting_id, &project_id) {
        (Some(m), _) => require_meeting(pool, m).await.is_ok(),
        (_, Some(p)) => find_project(pool, p).await.is_ok(),
        _ => false,
    };
    if !visible_source {
        return Err(format!("No document with id {id}"));
    }
    Ok(format!("# {title}\n\n_Written by {by}_\n\n{markdown}"))
}

pub async fn save_document(pool: &SqlitePool, args: &Value) -> Result<String, String> {
    if !has_table(pool, "documents").await {
        return Err("this Hearken database has no documents yet (open the latest Hearken once)".into());
    }
    let title = arg_str(args, "title").ok_or("title is required")?;
    let markdown = args.get("markdown").and_then(Value::as_str).map(str::trim).filter(|m| !m.is_empty())
        .ok_or("markdown is required")?;
    if markdown.to_lowercase().contains("<think") {
        return Err("the document must not contain <thinking> tags".into());
    }
    let (meeting_id, project_id, owner) = match (arg_str(args, "meeting_id"), arg_str(args, "project")) {
        (Some(m), None) => {
            let t = require_meeting(pool, m).await?;
            (Some(m.to_string()), None, format!("meeting \"{t}\""))
        }
        (None, Some(p)) => {
            let (pid, name) = find_project(pool, p).await?;
            (None, Some(pid), format!("project {name}"))
        }
        _ => return Err("give either meeting_id or project, not both".into()),
    };
    let id = format!("{:x}", {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (title, markdown.len(), now()).hash(&mut h);
        h.finish()
    });
    let ts = now();
    sqlx::query(
        "INSERT INTO documents (id, meeting_id, project_id, template_id, title, markdown, status, generated_by, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'claude', ?4, ?5, 'ready', 'claude-via-mcp', ?6, ?6)",
    )
    .bind(&id)
    .bind(meeting_id)
    .bind(project_id)
    .bind(title)
    .bind(markdown)
    .bind(&ts)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(format!("Saved \"{title}\" to {owner} [document {id}]. It appears under Documents in Hearken, where it can be edited or exported to Word or PDF."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_formats() {
        assert_eq!(clock(5.4), "00:05");
        assert_eq!(clock(754.0), "12:34");
        assert_eq!(clock(3725.0), "01:02:05");
    }

    #[test]
    fn fts_query_quotes_terms_and_prefixes_last() {
        assert_eq!(build_fts_query("budget rev").unwrap(), "\"budget\" AND \"rev\"*");
        assert_eq!(build_fts_query("say \"hi\"").unwrap(), "\"say\" AND \"hi\"*");
        assert!(build_fts_query("  -- ").is_none());
    }

    #[test]
    fn summary_body_strips_title_and_rejects_reasoning() {
        assert_eq!(summary_body("# Budget review\n\n## Key points\n- A").unwrap(), "## Key points\n- A");
        assert_eq!(summary_body("## Key points\n- A").unwrap(), "## Key points\n- A");
        assert!(summary_body("   ").is_err());
        assert!(summary_body("# Only a title").is_err());
        assert!(summary_body("<thinking>x</thinking>## A").is_err());
    }
}
