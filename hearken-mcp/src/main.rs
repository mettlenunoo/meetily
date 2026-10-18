//! hearken-mcp: a read-only MCP server over Hearken's local meeting database.
//!
//! Speaks MCP (JSON-RPC 2.0, one message per line) on stdin/stdout so MCP clients such as
//! Claude Desktop and Claude Code can search meetings, read summaries and transcripts, and
//! list action items. The database is opened read-only; this process never writes to it,
//! never touches audio, and makes no network calls. Logs go to stderr.
//!
//! Database: `$HEARKEN_DB`, else `--db <path>`, else
//! `~/Library/Application Support/com.mettlenunoo.hearken/meeting_minutes.sqlite`.

mod db;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const SERVER_NAME: &str = "hearken";
const LATEST_PROTOCOL: &str = "2025-06-18";
const SUPPORTED_PROTOCOLS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Hearken is the user's private, local meeting recorder. These tools read its \
meeting database: titles, AI summaries, the user's notes, timestamped transcripts, and extracted \
action items and decisions. Search first, then fetch the meeting you need. When you answer from a \
meeting, cite its title, date and the [mm:ss] timestamp. Items marked unverified have a supporting \
quote that was not found in the transcript; treat them with caution. Owners and due dates are only \
present when someone said them; never fill them in. If nothing relevant is found, say so instead of guessing.";

fn tools() -> Value {
    json!([
        {
            "name": "search_meetings",
            "description": "Full-text search across all Hearken meetings: titles, transcripts, summaries, notes, action items and decisions. Returns ranked matches with the meeting, where the match came from, a highlighted snippet and, for transcript matches, the [mm:ss] time in the recording. Supports prefix matching on the last word.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Words to search for."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "Maximum results (default 20)."}
                },
                "required": ["query"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "list_meetings",
            "description": "List Hearken meetings, newest first, with date, length, whether a summary exists and how many action items are still open.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Maximum meetings (default 20)."},
                    "title_contains": {"type": "string", "description": "Only meetings whose title contains this text (case-insensitive)."},
                    "since": {"type": "string", "description": "Only meetings on or after this date (YYYY-MM-DD)."}
                },
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "get_meeting",
            "description": "Get one meeting: title, date, AI summary, the user's notes, and its action items and decisions with evidence quotes and timestamps. Optionally includes the transcript with [mm:ss] times; use from/to (seconds) to fetch part of a long one.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "meeting_id": {"type": "string", "description": "Meeting id from search_meetings or list_meetings."},
                    "include_transcript": {"type": "boolean", "description": "Include the transcript (default false)."},
                    "from_seconds": {"type": "number", "minimum": 0, "description": "Transcript start, seconds from recording start."},
                    "to_seconds": {"type": "number", "minimum": 0, "description": "Transcript end, seconds from recording start."}
                },
                "required": ["meeting_id"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "list_action_items",
            "description": "List action items (and optionally decisions) across all meetings, newest meeting first, with owner and due date when someone stated them, status, the supporting quote and its [mm:ss] time.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": {"type": "string", "enum": ["open", "done", "dismissed", "any"], "description": "Default open."},
                    "kind": {"type": "string", "enum": ["action", "decision", "any"], "description": "Default action."},
                    "owner_contains": {"type": "string", "description": "Only items whose owner contains this text."},
                    "meeting_id": {"type": "string", "description": "Only items from this meeting."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500, "description": "Maximum items (default 100)."}
                },
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        }
    ])
}

async fn handle(pool: &sqlx::SqlitePool, method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(LATEST_PROTOCOL);
            let version = if SUPPORTED_PROTOCOLS.contains(&requested) { requested } else { LATEST_PROTOCOL };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": SERVER_NAME, "title": "Hearken meetings", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let result = match name {
                "search_meetings" => db::search_meetings(pool, &args).await,
                "list_meetings" => db::list_meetings(pool, &args).await,
                "get_meeting" => db::get_meeting(pool, &args).await,
                "list_action_items" => db::list_action_items(pool, &args).await,
                _ => return Err((-32602, format!("Unknown tool: {name}"))),
            };
            // Tool failures are reported in-band so the model can see and recover from them.
            Ok(match result {
                Ok(text) => json!({"content": [{"type": "text", "text": text}], "isError": false}),
                Err(e) => json!({"content": [{"type": "text", "text": format!("Error: {e}")}], "isError": true}),
            })
        }
        _ => Err((-32601, format!("Method not found: {method}"))),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let path = db::database_path();
    let pool = db::open_read_only(&path).await?;
    eprintln!("hearken-mcp: serving {}", path.display());

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("Parse error: {e}")}});
                stdout.write_all(format!("{err}\n").as_bytes()).await?;
                stdout.flush().await?;
                continue;
            }
        };
        // Notifications (no id) never get a response.
        let Some(id) = msg.get("id").cloned() else { continue };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        let response = match handle(&pool, method, &params).await {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        };
        stdout.write_all(format!("{response}\n").as_bytes()).await?;
        stdout.flush().await?;
    }
    Ok(())
}
