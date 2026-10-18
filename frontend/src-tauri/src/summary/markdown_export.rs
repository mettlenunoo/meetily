//! Exports a meeting as one Markdown file with YAML frontmatter:
//! summary, action items, decisions, the user's notes and the timestamped
//! transcript. Written next to the recording as `meeting.md` so the meeting
//! folder is readable without the app (Obsidian, grep, Claude, etc.).

use crate::state::AppState;
use sqlx::SqlitePool;
use std::path::PathBuf;
use tracing::{info, warn};

// The export itself lives in hearken-core so hearken-mcp writes the same file.
pub use hearken_core::export::{build_markdown, EXPORT_FILE_NAME};

/// Writes `meeting.md` into the meeting's recording folder (atomic replace).
/// Returns the path, or None when the meeting has no folder.
pub async fn write_to_meeting_folder(pool: &SqlitePool, meeting_id: &str) -> Result<Option<PathBuf>, String> {
    let path = hearken_core::export::write_to_meeting_folder(pool, meeting_id).await?;
    if let Some(p) = &path {
        info!("Wrote {}", p.display());
    }
    Ok(path)
}

/// Best-effort refresh used after summaries and item edits.
pub async fn refresh_quietly(pool: &SqlitePool, meeting_id: &str) {
    if let Err(e) = write_to_meeting_folder(pool, meeting_id).await {
        warn!("Could not write {} for {}: {}", EXPORT_FILE_NAME, meeting_id, e);
    }
}

/// Returns the meeting as Markdown (for copy / save-as in the UI).
#[tauri::command]
pub async fn api_export_meeting_markdown(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<String, String> {
    build_markdown(state.db_manager.pool(), &meeting_id).await.map(|(md, _)| md)
}

/// Writes `meeting.md` into the meeting folder and returns its path.
#[tauri::command]
pub async fn api_write_meeting_markdown(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Option<String>, String> {
    write_to_meeting_folder(state.db_manager.pool(), &meeting_id)
        .await
        .map(|p| p.map(|p| p.to_string_lossy().to_string()))
}
