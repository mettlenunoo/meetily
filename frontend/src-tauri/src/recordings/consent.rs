//! A record that the user told everyone they were recording. When a recording starts the
//! app shows a reminder with "I've told everyone"; pressing it is kept with the recording
//! and saved with the meeting. It never blocks or delays the recording itself.

use crate::state::AppState;
use chrono::Utc;
use sqlx::SqlitePool;
use std::sync::Mutex;

static PENDING: Mutex<Option<String>> = Mutex::new(None);

/// Called when a recording starts.
pub fn clear_pending() {
    *PENDING.lock().unwrap() = None;
}

pub async fn persist_pending(pool: &SqlitePool, meeting_id: &str) {
    let Some(at) = PENDING.lock().unwrap().take() else { return };
    if let Err(e) = sqlx::query("INSERT OR REPLACE INTO meeting_consent (meeting_id, confirmed_at) VALUES (?, ?)")
        .bind(meeting_id)
        .bind(&at)
        .execute(pool)
        .await
    {
        log::error!("Could not save the consent confirmation for {}: {}", meeting_id, e);
    }
}

#[tauri::command]
pub fn api_confirm_consent() -> Result<(), String> {
    if crate::audio::recording_commands::current_recording_seconds().is_none() {
        return Err("Not recording".into());
    }
    *PENDING.lock().unwrap() = Some(Utc::now().to_rfc3339());
    Ok(())
}

#[tauri::command]
pub async fn api_get_consent(state: tauri::State<'_, AppState>, meeting_id: String) -> Result<Option<String>, String> {
    sqlx::query_scalar("SELECT confirmed_at FROM meeting_consent WHERE meeting_id = ?")
        .bind(&meeting_id)
        .fetch_optional(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn confirmation_is_saved_with_the_meeting_once() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1', 'A', 'x', 'x')").execute(&pool).await.unwrap();
        clear_pending();
        persist_pending(&pool, "m1").await;
        let none: Option<String> = sqlx::query_scalar("SELECT confirmed_at FROM meeting_consent").fetch_optional(&pool).await.unwrap();
        assert!(none.is_none(), "nothing confirmed, nothing saved");
        *PENDING.lock().unwrap() = Some("2026-10-19T10:00:00Z".into());
        persist_pending(&pool, "m1").await;
        let at: String = sqlx::query_scalar("SELECT confirmed_at FROM meeting_consent WHERE meeting_id = 'm1'").fetch_one(&pool).await.unwrap();
        assert_eq!(at, "2026-10-19T10:00:00Z");
        assert!(PENDING.lock().unwrap().is_none(), "used once");
    }
}
