//! Recording types (meeting, lecture, interview, voice note), bookmarks of important
//! moments, and access to a meeting's audio files for the in-app player.

pub mod bookmarks;
pub mod consent;
pub mod speakers;
pub mod start_times;

use crate::state::AppState;
use serde::Serialize;
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Runtime};

pub const RECORDING_TYPES: &[&str] = &["meeting", "lecture", "interview", "voice_note"];

/// The template a recording type uses by default.
pub fn default_template(recording_type: &str) -> &'static str {
    match recording_type {
        "lecture" => "lecture_notes",
        "interview" => "interview",
        "voice_note" => "voice_note",
        _ => "standard_meeting",
    }
}

pub async fn recording_type(pool: &SqlitePool, meeting_id: &str) -> String {
    sqlx::query_scalar::<_, String>("SELECT recording_type FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "meeting".into())
}

pub async fn set_recording_type(pool: &SqlitePool, meeting_id: &str, recording_type: &str) -> Result<(), String> {
    if !RECORDING_TYPES.contains(&recording_type) {
        return Err(format!("Invalid recording type: {}", recording_type));
    }
    sqlx::query("UPDATE meetings SET recording_type = ? WHERE id = ?")
        .bind(recording_type)
        .bind(meeting_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn api_set_recording_type(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    recording_type: String,
) -> Result<(), String> {
    set_recording_type(state.db_manager.pool(), &meeting_id, &recording_type).await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    /// "mixed", "mic" (you) or "system" (everyone else)
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingMedia {
    pub recording_type: String,
    pub default_template: String,
    pub tracks: Vec<AudioTrack>,
    pub bookmarks: Vec<bookmarks::Bookmark>,
    /// Me / Others labels have been worked out for this meeting's transcript
    pub speakers_labeled: bool,
}

fn tracks_in(folder: &Path) -> Vec<AudioTrack> {
    [("mixed", "audio.mp4"), ("mic", "mic.mp4"), ("system", "system.mp4")]
        .iter()
        .map(|(kind, file)| (kind, folder.join(file)))
        .filter(|(_, p)| p.is_file())
        .map(|(kind, p)| AudioTrack { kind: kind.to_string(), path: p.to_string_lossy().into_owned() })
        .collect()
}

/// Recording type, bookmarks and audio files for one meeting. The audio files (and only
/// those) are added to the asset-protocol scope so the player can stream them.
#[tauri::command]
pub async fn api_get_meeting_media<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingMedia, String> {
    let pool = state.db_manager.pool();
    let folder: Option<String> = sqlx::query_scalar::<_, Option<String>>("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(&meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Meeting not found")?;
    let tracks = folder.map(PathBuf::from).filter(|f| f.is_dir()).map(|f| tracks_in(&f)).unwrap_or_default();
    let scope = app.asset_protocol_scope();
    for t in &tracks {
        if let Err(e) = scope.allow_file(&t.path) {
            log::warn!("Could not allow audio file for playback {}: {}", t.path, e);
        }
    }
    let recording_type = recording_type(pool, &meeting_id).await;
    let speakers_labeled: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM transcripts WHERE meeting_id = ? AND speaker IS NOT NULL)")
        .bind(&meeting_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(MeetingMedia {
        speakers_labeled,
        default_template: default_template(&recording_type).to_string(),
        recording_type,
        tracks,
        bookmarks: bookmarks::list(pool, &meeting_id).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_type_has_a_template() {
        assert_eq!(default_template("lecture"), "lecture_notes");
        assert_eq!(default_template("meeting"), "standard_meeting");
        assert_eq!(default_template("something else"), "standard_meeting");
        for t in RECORDING_TYPES {
            let id = default_template(t);
            assert!(
                crate::summary::templates::get_template(id).is_ok(),
                "template {id} for {t} exists and parses"
            );
        }
    }

    #[test]
    fn finds_only_existing_tracks() {
        let dir = std::env::temp_dir().join(format!("hearken-tracks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("audio.mp4"), b"x").unwrap();
        std::fs::write(dir.join("mic.mp4"), b"x").unwrap();
        let kinds: Vec<String> = tracks_in(&dir).into_iter().map(|t| t.kind).collect();
        assert_eq!(kinds, vec!["mixed", "mic"]);
        std::fs::remove_dir_all(dir).ok();
    }
}
