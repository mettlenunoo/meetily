//! hearken-mcp: an MCP server over Hearken's local meeting database.
//!
//! Speaks MCP (JSON-RPC 2.0, one message per line) on stdin/stdout so MCP clients such as
//! Claude Desktop and Claude Code can search meetings, read summaries and transcripts, and
//! list action items. Claude can also save two things back, so summaries can be written with
//! the user's Claude subscription instead of an API key:
//! - a meeting's summary (the previous one is kept in the app's backup column), and
//! - its AI action items / decisions, grounded by hearken-core exactly like the app's own
//!   extraction (quotes verified, times from the transcript, user-created/edited items kept).
//! Nothing else is ever written: no deletes, no transcript, notes or audio changes. No network
//! calls. Logs go to stderr.
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
meeting, cite its title, date and the [mm:ss] timestamp. Meetings are filed under projects (and \
courses); use list_projects / get_project for anything about a project, and list_action_items with \
due=overdue for what is late. To summarise a meeting: read it with \
get_meeting(include_transcript=true), paging through long transcripts, then call save_summary and \
save_meeting_items. Only save when the user asked you to summarise or save. Items marked unverified have a supporting \
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
            "description": "List action items (and optionally decisions) across all meetings, soonest due first, with owner and due date when someone stated them (the spoken words and the exact date when known), OVERDUE flags, status, the supporting quote and its [mm:ss] time, and each item's id for update_action_item.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": {"type": "string", "enum": ["open", "done", "dismissed", "any"], "description": "Default open."},
                    "kind": {"type": "string", "enum": ["action", "decision", "any"], "description": "Default action."},
                    "owner_contains": {"type": "string", "description": "Only items whose owner contains this text."},
                    "meeting_id": {"type": "string", "description": "Only items from this meeting."},
                    "project": {"type": "string", "description": "Only items from meetings in this project (name or id)."},
                    "due": {"type": "string", "enum": ["any", "overdue", "week", "undated"], "description": "overdue, due within 7 days, or no exact date. Default any."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500, "description": "Maximum items (default 100)."}
                },
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "save_summary",
            "description": "Save a Markdown summary for one meeting into Hearken, replacing the current one (which Hearken keeps as a backup). Only call this when the user asked you to summarise or save. Base it only on the meeting's transcript and notes. Do not start with a '# Title' line (Hearken shows the title separately); use '##' section headings, e.g. Overview, Key points, Decisions, Action items, Open questions. Refer to moments as [mm:ss].",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "meeting_id": {"type": "string"},
                    "markdown": {"type": "string", "description": "The summary in Markdown."}
                },
                "required": ["meeting_id", "markdown"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "save_meeting_items",
            "description": "Save the action items and decisions for one meeting. Replaces the meeting's previous AI-generated items; items the user created or edited in Hearken are never touched. Send the complete list every time. Rules: use only what the transcript says; owner and due are null unless someone stated them (no guessing); due is as spoken (e.g. 'Friday'); evidence_quote must be 5 to 25 consecutive words copied exactly from the transcript (one line, or across two adjacent lines), without the [mm:ss] prefix. Each quote is checked against the transcript and the time is taken from where it appears; the reply lists any that could not be verified so you can correct them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "meeting_id": {"type": "string"},
                    "action_items": {"type": "array", "items": {"$ref": "#/$defs/item"}},
                    "decisions": {"type": "array", "items": {"$ref": "#/$defs/item"}}
                },
                "required": ["meeting_id", "action_items", "decisions"],
                "additionalProperties": false,
                "$defs": {
                    "item": {
                        "type": "object",
                        "properties": {
                            "title": {"type": "string", "description": "Short imperative for actions (\"Send the revised forecast\"); a statement for decisions."},
                            "description": {"type": ["string", "null"]},
                            "owner": {"type": ["string", "null"], "description": "As named in the transcript, or null."},
                            "due": {"type": ["string", "null"], "description": "As spoken, or null."},
                            "priority": {"type": ["string", "null"], "enum": ["high", "medium", "low", null]},
                            "evidence_quote": {"type": "string"}
                        },
                        "required": ["title", "evidence_quote"],
                        "additionalProperties": false
                    }
                }
            },
        },
        {
            "name": "list_projects",
            "description": "List the user's projects and courses with how many meetings, open and overdue action items each has.",
            "inputSchema": {
                "type": "object",
                "properties": {"include_archived": {"type": "boolean", "description": "Default false."}},
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "get_project",
            "description": "One project: description, people, keywords, its meetings, and the action items and decisions from them (open first, soonest due first).",
            "inputSchema": {
                "type": "object",
                "properties": {"project": {"type": "string", "description": "Project name or id."}},
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "assign_meeting_to_project",
            "description": "File a meeting under a project, as its main project (the previous main one stays linked as an extra) or as an extra project. Only when the user asks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "meeting_id": {"type": "string"},
                    "project": {"type": "string", "description": "Project name or id."},
                    "as": {"type": "string", "enum": ["main", "extra"], "description": "Default main."}
                },
                "required": ["meeting_id", "project"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "update_action_item",
            "description": "Change an action item when the user asks: mark it done (or open again, or dismissed), set or clear the owner, set the due date, or rename it. Give only what changes; an empty string clears owner or due_date. Changing content marks the item as the user's, so later AI re-extraction keeps it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "item_id": {"type": "string", "description": "From list_action_items, get_meeting or get_project ([item ...])."},
                    "status": {"type": "string", "enum": ["open", "done", "dismissed"]},
                    "title": {"type": "string"},
                    "owner": {"type": "string"},
                    "due_date": {"type": "string", "description": "YYYY-MM-DD; work it out from today's date and tell the user which date you set."}
                },
                "required": ["item_id"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "get_document",
            "description": "Read a document (minutes, report, lecture notes, status report...) listed under a meeting or project.",
            "inputSchema": {
                "type": "object",
                "properties": {"document_id": {"type": "string"}},
                "required": ["document_id"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "save_document",
            "description": "Save a document you wrote (e.g. minutes, a formal report, a project status report, a study guide) into Hearken under one meeting or one project, when the user asks you to. It is added next to the summary; nothing is replaced. Base it on Hearken's data, use ## headings, cite [mm:ss] or meeting dates, and fill owners and dates only when recorded.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "markdown": {"type": "string"},
                    "meeting_id": {"type": "string", "description": "Save under this meeting..."},
                    "project": {"type": "string", "description": "...or under this project (name or id)."}
                },
                "required": ["title", "markdown"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false}
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
                "capabilities": {"tools": {"listChanged": false}, "prompts": {"listChanged": false}},
                "serverInfo": {"name": SERVER_NAME, "title": "Hearken meetings", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "prompts/list" => Ok(json!({"prompts": [{
            "name": "summarise_meeting",
            "title": "Summarise a Hearken meeting",
            "description": "Summarise a meeting and save the summary, action items and decisions back into Hearken.",
            "arguments": [{"name": "meeting", "description": "Meeting title or id (leave empty for the most recent).", "required": false}]
        }]})),
        "prompts/get" => {
            if params.get("name").and_then(Value::as_str) != Some("summarise_meeting") {
                return Err((-32602, "Unknown prompt".into()));
            }
            let which = params
                .pointer("/arguments/meeting")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|m| format!("the Hearken meeting \"{m}\" (find it with list_meetings or search_meetings)"))
                .unwrap_or_else(|| "my most recent Hearken meeting (use list_meetings)".into());
            Ok(json!({
                "description": "Summarise a Hearken meeting and save it",
                "messages": [{"role": "user", "content": {"type": "text", "text": format!(
                    "Summarise {which}.\n\n\
                     1. Read it with get_meeting(include_transcript=true); if the transcript is truncated, keep paging with from_seconds until you have all of it.\n\
                     2. Write a clear summary in Markdown with ## sections (Overview, Key points, Decisions, Action items, Open questions), citing [mm:ss] for important moments, and save it with save_summary.\n\
                     3. Extract every action item and decision and save them with save_meeting_items. Owners and due dates only if someone said them. Each evidence_quote copied exactly from the transcript.\n\
                     4. If any quotes come back unverified, fix them and save again.\n\
                     5. Then show me the summary and the items."
                )}}]
            }))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let result = match name {
                "search_meetings" => db::search_meetings(pool, &args).await,
                "list_meetings" => db::list_meetings(pool, &args).await,
                "get_meeting" => db::get_meeting(pool, &args).await,
                "list_action_items" => db::list_action_items(pool, &args).await,
                "list_projects" => db::list_projects(pool, &args).await,
                "get_project" => db::get_project(pool, &args).await,
                "assign_meeting_to_project" => db::assign_meeting_to_project(pool, &args).await,
                "update_action_item" => db::update_action_item(pool, &args).await,
                "get_document" => db::get_document(pool, &args).await,
                "save_document" => db::save_document(pool, &args).await,
                "save_summary" => db::save_summary(pool, &args).await,
                "save_meeting_items" => db::save_meeting_items(pool, &args).await,
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
    let pool = db::open(&path).await?;
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
