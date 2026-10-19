//! The link to Desmond's Slack bot. Hearken runs the bot's `hearken_bridge.py`, either on
//! this Mac or over SSH on the VPS (using the user's SSH keys), and exchanges JSON. No
//! server, port or account is involved, and nothing is sent until the user chooses to:
//!
//! - projects (never local-only ones) and their Slack channels are copied to the bot;
//! - "Send to Slack" shares one meeting: summary, action items and decisions. Never the
//!   transcript or audio, and never a meeting in a local-only project;
//! - "Refresh" copies the bot's tasks and decisions per project back for project pages.

use crate::state::AppState;
use chrono::Utc;
use hearken_core::export::summary_markdown;
use hearken_core::items::format_clock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, SqlitePool};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

const CONFIG_FILE: &str = "slack_bridge.json";
const TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotLink {
    /// "local" (the bot's folder on this Mac) or "ssh" (on a server)
    pub mode: String,
    /// The bot's Python, e.g. ~/Documents/GitHub/desmonds-bot/.venv/bin/python
    pub python: String,
    /// Path to hearken_bridge.py
    pub script: String,
    /// For "ssh": the host as in ~/.ssh/config, e.g. "my-vps" or "user@1.2.3.4"
    pub ssh_host: Option<String>,
}

impl Default for BotLink {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let repo = format!("{home}/Documents/GitHub/desmonds-bot");
        BotLink { mode: "local".into(), python: format!("{repo}/.venv/bin/python"), script: format!("{repo}/hearken_bridge.py"), ssh_host: None }
    }
}

fn config_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join(CONFIG_FILE))
}

pub fn load_link<R: Runtime>(app: &AppHandle<R>) -> Result<Option<BotLink>, String> {
    let path = config_path(app)?;
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map(Some).map_err(|e| e.to_string())
}

/// Single-quotes a word for the remote shell that ssh runs the command in.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The command that runs `hearken_bridge.py <command>`.
fn bridge_command(link: &BotLink, command: &str) -> Result<Command, String> {
    match link.mode.as_str() {
        "local" => {
            let mut c = Command::new(&link.python);
            c.arg(&link.script).arg(command);
            Ok(c)
        }
        "ssh" => {
            let host = link.ssh_host.as_deref().map(str::trim).filter(|h| !h.is_empty()).ok_or("Set the SSH host")?;
            let mut c = Command::new("/usr/bin/ssh");
            c.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host])
                .arg(format!("{} {} {}", shell_quote(&link.python), shell_quote(&link.script), shell_quote(command)));
            Ok(c)
        }
        other => Err(format!("Unknown connection mode: {}", other)),
    }
}

/// Runs one bridge command and returns its JSON answer (or its error message).
pub async fn run_bridge(link: &BotLink, command: &str, input: Option<Value>) -> Result<Value, String> {
    let mut cmd = bridge_command(link, command)?;
    let label = command.to_string();
    let output = tokio::task::spawn_blocking(move || -> Result<std::process::Output, String> {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("Could not start the bot bridge: {}", e))?;
        if let Some(mut stdin) = child.stdin.take() {
            let body = input.map(|v| v.to_string()).unwrap_or_else(|| "{}".into());
            let _ = stdin.write_all(body.as_bytes());
        }
        let deadline = std::time::Instant::now() + TIMEOUT;
        loop {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                return child.wait_with_output().map_err(|e| e.to_string());
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                return Err(format!("The bot didn't answer within {} s ({})", TIMEOUT.as_secs(), label));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })
    .await
    .map_err(|e| e.to_string())??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let last = stdout.lines().rev().find(|l| l.trim_start().starts_with('{'));
    match last.and_then(|l| serde_json::from_str::<Value>(l).ok()) {
        Some(v) if v.get("error").is_some() => Err(v["error"].as_str().unwrap_or("The bot reported an error").to_string()),
        Some(v) if output.status.success() => Ok(v),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("The bot bridge failed: {}", stderr.lines().last().unwrap_or("no output").trim()))
        }
    }
}

async fn require_link<R: Runtime>(app: &AppHandle<R>) -> Result<BotLink, String> {
    load_link(app)?.ok_or_else(|| "Connect Hearken to the Slack bot in Settings first".to_string())
}

// ---------------------------------------------------------------------------
// Packets
// ---------------------------------------------------------------------------

/// Projects the bot may know about: active and not local-only.
async fn shareable_projects(pool: &SqlitePool) -> Result<Value, String> {
    let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, name, kind, slack_channels FROM projects WHERE status = 'active' AND privacy != 'local' ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(json!({
        "projects": rows.into_iter().map(|(id, name, kind, channels)| json!({
            "id": id, "name": name, "kind": kind,
            "slack_channels": channels.unwrap_or_default().split(',').map(str::trim).filter(|c| !c.is_empty()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>()
    }))
}

fn content_hash(v: &Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.to_string().hash(&mut h);
    format!("{:016x}", h.finish())
}

/// What "Send to Slack" sends for a meeting. The transcript and audio are never included.
pub async fn meeting_packet(pool: &SqlitePool, meeting_id: &str) -> Result<Value, String> {
    if crate::projects::is_local_only(pool, meeting_id).await {
        return Err("This meeting is in a local-only project, so it can't be shared to Slack".into());
    }
    let (title, created): (String, String) = sqlx::query_as("SELECT title, created_at FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Meeting not found")?;
    let date = hearken_core::items::parse_day(&created).map(|d| d.format("%a %-d %b %Y").to_string()).unwrap_or_default();
    let summary = sqlx::query_scalar::<_, Option<String>>("SELECT result FROM summary_processes WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .flatten()
        .and_then(|r| summary_markdown(&r));
    let project = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT p.id, p.name, p.slack_channels FROM meeting_projects mp JOIN projects p ON p.id = mp.project_id
         WHERE mp.meeting_id = ? AND mp.is_primary = 1",
    )
    .bind(meeting_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .map(|(id, name, channels)| json!({
        "id": id, "name": name,
        "slack_channels": channels.unwrap_or_default().split(',').map(str::trim).filter(|c| !c.is_empty()).collect::<Vec<_>>(),
    }));
    let items: Vec<(String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<f64>)> = sqlx::query_as(
        "SELECT kind, title, owner, due_at, due_date, evidence_quote, evidence_start_time FROM meeting_items
         WHERE meeting_id = ? AND status = 'open' ORDER BY kind, COALESCE(evidence_start_time, 1e12)",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let (mut actions, mut decisions) = (Vec::new(), Vec::new());
    for (kind, title, owner, due_at, due_words, quote, at) in items {
        let at = at.map(format_clock);
        if kind == "action" {
            actions.push(json!({"title": title, "owner": owner, "due_at": due_at, "due_words": due_words, "quote": quote, "at": at}));
        } else {
            decisions.push(json!({"title": title, "at": at}));
        }
    }
    let content = json!({
        "meeting": {"id": meeting_id, "title": title, "date": date},
        "project": project,
        "summary_markdown": summary,
        "action_items": actions,
        "decisions": decisions,
    });
    let packet_id = format!("{}:{}", meeting_id, content_hash(&content));
    let mut packet = content;
    packet["packet_id"] = json!(packet_id);
    Ok(packet)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotLinkState {
    pub link: BotLink,
    pub configured: bool,
}

#[tauri::command]
pub fn api_get_bot_link<R: Runtime>(app: AppHandle<R>) -> Result<BotLinkState, String> {
    Ok(match load_link(&app)? {
        Some(link) => BotLinkState { link, configured: true },
        None => BotLinkState { link: BotLink::default(), configured: false },
    })
}

#[tauri::command]
pub fn api_save_bot_link<R: Runtime>(app: AppHandle<R>, link: BotLink) -> Result<(), String> {
    if !matches!(link.mode.as_str(), "local" | "ssh") {
        return Err("Choose 'On this Mac' or 'On a server (SSH)'".into());
    }
    let path = config_path(&app)?;
    std::fs::write(&path, serde_json::to_string_pretty(&link).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Checks the connection: {ok, slack, projects}.
#[tauri::command]
pub async fn api_test_bot_link<R: Runtime>(app: AppHandle<R>, link: Option<BotLink>) -> Result<Value, String> {
    let link = match link {
        Some(l) => l,
        None => require_link(&app).await?,
    };
    run_bridge(&link, "ping", None).await
}

/// Copies projects (not local-only) and their channels to the bot.
#[tauri::command]
pub async fn api_sync_bot_projects<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let link = require_link(&app).await?;
    let projects = shareable_projects(state.db_manager.pool()).await?;
    run_bridge(&link, "sync-projects", Some(projects)).await
}

#[tauri::command]
pub async fn api_preview_meeting_share(state: tauri::State<'_, AppState>, meeting_id: String) -> Result<Value, String> {
    meeting_packet(state.db_manager.pool(), &meeting_id).await
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SlackShare {
    pub packet_id: String,
    pub meeting_id: String,
    pub channel: String,
    pub ts: Option<String>,
    pub permalink: Option<String>,
    pub tasks_proposed: i64,
    pub decisions_saved: i64,
    pub shared_at: String,
}

#[tauri::command]
pub async fn api_list_meeting_shares(state: tauri::State<'_, AppState>, meeting_id: String) -> Result<Vec<SlackShare>, String> {
    sqlx::query_as::<_, SlackShare>("SELECT * FROM slack_shares WHERE meeting_id = ? ORDER BY shared_at DESC")
        .bind(&meeting_id)
        .fetch_all(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())
}

/// Shares the meeting (as previewed) to `channel`, after refreshing the bot's projects.
#[tauri::command]
pub async fn api_share_meeting<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    channel: String,
) -> Result<SlackShare, String> {
    let pool = state.db_manager.pool();
    let link = require_link(&app).await?;
    let mut packet = meeting_packet(pool, &meeting_id).await?;
    let channel = channel.trim().to_string();
    if channel.is_empty() {
        return Err("Choose a Slack channel".into());
    }
    packet["channel"] = json!(channel);
    let projects = shareable_projects(pool).await?;
    run_bridge(&link, "sync-projects", Some(projects)).await?;
    let result = run_bridge(&link, "import-meeting", Some(packet.clone())).await?;

    let share = SlackShare {
        packet_id: packet["packet_id"].as_str().unwrap_or_default().to_string(),
        meeting_id: meeting_id.clone(),
        channel,
        ts: result["ts"].as_str().map(str::to_string),
        permalink: result["permalink"].as_str().map(str::to_string),
        tasks_proposed: result["tasks_proposed"].as_i64().unwrap_or(0),
        decisions_saved: result["decisions_saved"].as_i64().unwrap_or(0),
        shared_at: Utc::now().to_rfc3339(),
    };
    sqlx::query(
        "INSERT OR REPLACE INTO slack_shares (packet_id, meeting_id, channel, ts, permalink, tasks_proposed, decisions_saved, shared_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&share.packet_id)
    .bind(&share.meeting_id)
    .bind(&share.channel)
    .bind(&share.ts)
    .bind(&share.permalink)
    .bind(share.tasks_proposed)
    .bind(share.decisions_saved)
    .bind(&share.shared_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(share)
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SlackItem {
    pub key: String,
    pub project_id: String,
    pub kind: String,
    pub title: String,
    pub status: Option<String>,
    pub assignee: Option<String>,
    pub due_at: Option<String>,
    pub overdue: bool,
    pub link: Option<String>,
    pub source: Option<String>,
    pub meeting_ref: Option<String>,
    pub updated_at: Option<String>,
    pub synced_at: String,
}

pub async fn store_export(pool: &SqlitePool, export: &Value) -> Result<usize, String> {
    let now = Utc::now().to_rfc3339();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM slack_items").execute(&mut *tx).await.map_err(|e| e.to_string())?;
    let mut n = 0;
    let empty = vec![];
    for t in export["tasks"].as_array().unwrap_or(&empty) {
        sqlx::query(
            "INSERT INTO slack_items (key, project_id, kind, title, status, assignee, due_at, overdue, updated_at, synced_at)
             VALUES (?, ?, 'task', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("task:{}", t["id"]))
        .bind(t["project_id"].as_str().unwrap_or_default())
        .bind(t["title"].as_str().unwrap_or_default())
        .bind(t["status"].as_str())
        .bind(t["assignee"].as_str())
        .bind(t["due_at"].as_str())
        .bind(t["overdue"].as_bool().unwrap_or(false))
        .bind(t["updated_at"].as_str())
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        n += 1;
    }
    for d in export["decisions"].as_array().unwrap_or(&empty) {
        sqlx::query(
            "INSERT INTO slack_items (key, project_id, kind, title, link, source, meeting_ref, updated_at, synced_at)
             VALUES (?, ?, 'decision', ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("decision:{}", d["id"]))
        .bind(d["project_id"].as_str().unwrap_or_default())
        .bind(d["text"].as_str().unwrap_or_default())
        .bind(d["link"].as_str())
        .bind(d["source"].as_str())
        .bind(d["meeting_ref"].as_str())
        .bind(d["created_at"].as_str())
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        n += 1;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(n)
}

/// Copies the bot's tasks and decisions (per project) into Hearken.
#[tauri::command]
pub async fn api_pull_from_bot<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>) -> Result<usize, String> {
    let link = require_link(&app).await?;
    let export = run_bridge(&link, "export", None).await?;
    store_export(state.db_manager.pool(), &export).await
}

#[tauri::command]
pub async fn api_list_slack_items(state: tauri::State<'_, AppState>, project_id: String) -> Result<Vec<SlackItem>, String> {
    sqlx::query_as::<_, SlackItem>(
        "SELECT * FROM slack_items WHERE project_id = ?
         ORDER BY kind DESC, (status = 'done'), overdue DESC, due_at IS NULL, due_at, updated_at DESC",
    )
    .bind(&project_id)
    .fetch_all(state.db_manager.pool())
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_for_the_remote_shell() {
        assert_eq!(shell_quote("/opt/bot/.venv/bin/python"), "'/opt/bot/.venv/bin/python'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn ssh_mode_builds_one_quoted_remote_command() {
        let link = BotLink { mode: "ssh".into(), python: "/opt/b/py".into(), script: "/opt/b/hearken bridge.py".into(), ssh_host: Some("vps".into()) };
        let cmd = bridge_command(&link, "ping").unwrap();
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(cmd.get_program(), "/usr/bin/ssh");
        assert!(args.contains(&"BatchMode=yes".to_string()));
        assert_eq!(args.last().unwrap(), "'/opt/b/py' '/opt/b/hearken bridge.py' 'ping'");
        let no_host = BotLink { ssh_host: None, ..link };
        assert!(bridge_command(&no_host, "ping").is_err());
    }

    /// Talks to a real bot when HEARKEN_TEST_BOT_PYTHON and HEARKEN_TEST_BOT_SCRIPT are set.
    #[tokio::test]
    async fn real_bridge_round_trip_when_available() {
        let (Ok(python), Ok(script)) = (std::env::var("HEARKEN_TEST_BOT_PYTHON"), std::env::var("HEARKEN_TEST_BOT_SCRIPT")) else {
            return;
        };
        let link = BotLink { mode: "local".into(), python, script, ssh_host: None };
        let ping = run_bridge(&link, "ping", None).await.unwrap();
        assert_eq!(ping["ok"], true);
        let synced = run_bridge(&link, "sync-projects", Some(json!({"projects": [{"id": "t1", "name": "Test", "slack_channels": []}]}))).await.unwrap();
        assert_eq!(synced["projects"], 1);
        let exported = run_bridge(&link, "export", None).await.unwrap();
        assert!(exported["tasks"].is_array());
        let err = run_bridge(&link, "import-meeting", Some(json!({}))).await.unwrap_err();
        assert!(!err.is_empty(), "errors come back as messages: {err}");
    }

    async fn db() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1', 'Geoportal weekly', '2026-10-14T10:00:00Z', 'x')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time) VALUES ('t1', 'm1', 'Secret transcript words', 'x', 1)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name, slack_channels, created_at, updated_at) VALUES ('p1', 'IFDC Geoportal', '#ifdc-geoportal, #gis', 'x', 'x')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name, privacy, created_at, updated_at) VALUES ('p2', 'Private', 'local', 'x', 'x')")
            .execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn packet_has_summary_and_items_but_never_the_transcript() {
        let pool = db().await;
        crate::projects::set_meeting_projects(&pool, "m1", Some("p1"), &[]).await.unwrap();
        sqlx::query("INSERT INTO summary_processes (meeting_id, status, created_at, updated_at, result) VALUES ('m1', 'completed', 'x', 'x', '{\"markdown\":\"## Overview\\nAgreed.\"}')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO meeting_items (id, meeting_id, kind, title, owner, due_at, due_date, status, source, evidence_verified, evidence_quote, evidence_start_time, created_at, updated_at) VALUES
            ('i1', 'm1', 'action', 'Send layers', 'Ama', '2026-10-16', 'Friday', 'open', 'ai', 1, 'Ama will send the layers', 310, 'x', 'x'),
            ('i2', 'm1', 'decision', 'Use OSM', NULL, NULL, NULL, 'open', 'ai', 1, NULL, 754, 'x', 'x'),
            ('i3', 'm1', 'action', 'Done already', NULL, NULL, NULL, 'done', 'ai', 1, NULL, NULL, 'x', 'x')")
            .execute(&pool).await.unwrap();
        let p = meeting_packet(&pool, "m1").await.unwrap();
        let text = p.to_string();
        assert!(!text.contains("Secret transcript words"), "no transcript");
        assert_eq!(p["meeting"]["date"], "Wed 14 Oct 2026");
        assert_eq!(p["project"]["slack_channels"], json!(["#ifdc-geoportal", "#gis"]));
        assert_eq!(p["summary_markdown"], "## Overview\nAgreed.");
        assert_eq!(p["action_items"].as_array().unwrap().len(), 1, "only open actions");
        assert_eq!(p["action_items"][0]["due_at"], "2026-10-16");
        assert_eq!(p["action_items"][0]["at"], "05:10");
        assert_eq!(p["decisions"][0], json!({"title": "Use OSM", "at": "12:34"}));
        let again = meeting_packet(&pool, "m1").await.unwrap();
        assert_eq!(p["packet_id"], again["packet_id"], "same content, same packet id");
        sqlx::query("UPDATE meeting_items SET title = 'Send all layers' WHERE id = 'i1'").execute(&pool).await.unwrap();
        assert_ne!(p["packet_id"], meeting_packet(&pool, "m1").await.unwrap()["packet_id"], "changed content can be shared again");
    }

    #[tokio::test]
    async fn local_only_meetings_and_projects_are_never_shared() {
        let pool = db().await;
        crate::projects::set_meeting_projects(&pool, "m1", Some("p1"), &["p2".into()]).await.unwrap();
        assert!(meeting_packet(&pool, "m1").await.unwrap_err().contains("local-only"));
        let projects = shareable_projects(&pool).await.unwrap().to_string();
        assert!(projects.contains("IFDC Geoportal") && !projects.contains("Private"));
    }

    #[tokio::test]
    async fn export_replaces_slack_items() {
        let pool = db().await;
        let export = json!({
            "tasks": [{"id": 12, "project_id": "p1", "title": "Send layers", "status": "todo", "assignee": "Ama", "due_at": "2026-10-16", "overdue": true, "updated_at": "x"}],
            "decisions": [{"id": 3, "project_id": "p1", "text": "Use OSM", "link": "https://slack/x", "source": "meeting", "meeting_ref": "Weekly [12:34]", "created_at": "x"}]
        });
        assert_eq!(store_export(&pool, &export).await.unwrap(), 2);
        assert_eq!(store_export(&pool, &export).await.unwrap(), 2, "a refresh replaces, never duplicates");
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM slack_items").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 2);
    }
}
