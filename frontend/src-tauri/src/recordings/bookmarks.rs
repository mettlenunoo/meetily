//! Bookmarks: moments marked as important. During a recording they are kept in memory with
//! the recording time (pauses excluded, so they line up with transcript times) and saved
//! with the meeting when the recording is saved. Afterwards they can be added from the
//! player. Summaries are told about them so marked moments get prominence.

use crate::state::AppState;
use chrono::Utc;
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Runtime};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    pub meeting_id: String,
    pub seconds: f64,
    pub note: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveBookmark {
    pub seconds: f64,
    pub note: Option<String>,
}

/// Bookmarks made during the current recording, not yet attached to a meeting.
static PENDING: Mutex<Vec<LiveBookmark>> = Mutex::new(Vec::new());

/// Called when a recording starts.
pub fn clear_pending() {
    PENDING.lock().unwrap().clear();
}

fn clean_note(note: Option<String>) -> Option<String> {
    note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty())
}

/// Marks "now" in the current recording. Fails when nothing is recording.
pub fn add_live<R: Runtime>(app: &AppHandle<R>, note: Option<String>) -> Result<LiveBookmark, String> {
    let seconds = crate::audio::recording_commands::current_recording_seconds()
        .ok_or("Not recording, so there's nothing to bookmark")?;
    let bookmark = LiveBookmark { seconds, note: clean_note(note) };
    PENDING.lock().unwrap().push(bookmark.clone());
    let _ = app.emit("bookmark-added", &bookmark);
    log::info!("Bookmarked {:.1}s of the current recording", seconds);
    Ok(bookmark)
}

/// Attaches this recording's bookmarks to the meeting it was saved as.
pub async fn persist_pending(pool: &SqlitePool, meeting_id: &str) {
    let pending: Vec<LiveBookmark> = std::mem::take(&mut *PENDING.lock().unwrap());
    for b in pending {
        if let Err(e) = insert(pool, meeting_id, b.seconds, b.note).await {
            log::error!("Could not save a bookmark for {}: {}", meeting_id, e);
        }
    }
}

async fn insert(pool: &SqlitePool, meeting_id: &str, seconds: f64, note: Option<String>) -> Result<Bookmark, String> {
    let b = Bookmark {
        id: Uuid::new_v4().to_string(),
        meeting_id: meeting_id.to_string(),
        seconds: seconds.max(0.0),
        note: clean_note(note),
        created_at: Utc::now().to_rfc3339(),
    };
    sqlx::query("INSERT INTO meeting_bookmarks (id, meeting_id, seconds, note, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&b.id)
        .bind(&b.meeting_id)
        .bind(b.seconds)
        .bind(&b.note)
        .bind(&b.created_at)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(b)
}

pub async fn list(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<Bookmark>, String> {
    sqlx::query_as::<_, Bookmark>("SELECT * FROM meeting_bookmarks WHERE meeting_id = ? ORDER BY seconds")
        .bind(meeting_id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())
}

/// "[12:34] note" lines for the summary prompt, or None when there are no bookmarks.
pub async fn prompt_lines(pool: &SqlitePool, meeting_id: &str) -> Option<String> {
    let marks = list(pool, meeting_id).await.ok()?;
    if marks.is_empty() {
        return None;
    }
    Some(
        marks
            .iter()
            .map(|b| {
                let at = hearken_core::items::format_clock(b.seconds);
                match &b.note {
                    Some(n) => format!("- [{}] {}", at, n),
                    None => format!("- [{}]", at),
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[tauri::command]
pub fn api_add_live_bookmark<R: Runtime>(app: AppHandle<R>, note: Option<String>) -> Result<LiveBookmark, String> {
    add_live(&app, note)
}

#[tauri::command]
pub fn api_get_live_bookmarks() -> Vec<LiveBookmark> {
    PENDING.lock().unwrap().clone()
}

#[tauri::command]
pub async fn api_add_bookmark(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    seconds: f64,
    note: Option<String>,
) -> Result<Bookmark, String> {
    insert(state.db_manager.pool(), &meeting_id, seconds, note).await
}

#[tauri::command]
pub async fn api_delete_bookmark(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    sqlx::query("DELETE FROM meeting_bookmarks WHERE id = ?")
        .bind(&id)
        .execute(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_notes_become_none() {
        assert_eq!(clean_note(Some("  ".into())), None);
        assert_eq!(clean_note(Some(" exam topic ".into())).as_deref(), Some("exam topic"));
    }

    #[tokio::test]
    async fn pending_bookmarks_attach_to_the_saved_meeting() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1', 'Lecture 3', 'x', 'x')")
            .execute(&pool).await.unwrap();
        clear_pending();
        PENDING.lock().unwrap().push(LiveBookmark { seconds: 754.2, note: Some("exam".into()) });
        PENDING.lock().unwrap().push(LiveBookmark { seconds: 65.0, note: None });
        persist_pending(&pool, "m1").await;
        assert!(PENDING.lock().unwrap().is_empty());
        let marks = list(&pool, "m1").await.unwrap();
        assert_eq!(marks.iter().map(|b| b.seconds).collect::<Vec<_>>(), vec![65.0, 754.2]);
        assert_eq!(prompt_lines(&pool, "m1").await.unwrap(), "- [01:05]\n- [12:34] exam");
        let rt: String = sqlx::query_scalar("SELECT recording_type FROM meetings WHERE id = 'm1'").fetch_one(&pool).await.unwrap();
        assert_eq!(rt, "meeting", "default recording type");
    }
}
