//! Hearken Inbox: recordings made anywhere (iPhone Voice Memos, an iPad, a recorder app)
//! and saved into a folder, by default "Hearken Inbox" in iCloud Drive, are imported and
//! transcribed automatically, like a recording made on the Mac. Nothing new runs online:
//! Apple's iCloud Drive moves the file, everything else happens here.
//!
//! Files are never deleted: after import they're moved to "Imported/", or to
//! "Couldn't import/" if that failed. A file is only picked up once it has stopped
//! changing (iCloud may still be syncing it), and never while you're recording.

use crate::audio::constants::AUDIO_EXTENSIONS;
use crate::database::repositories::setting::SettingsRepository;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

const SETTINGS_FILE: &str = "inbox.json";
const SCAN_EVERY: Duration = Duration::from_secs(45);
/// A file must be untouched for this long before it's imported.
const SETTLE: Duration = Duration::from_secs(20);
pub const IMPORTED_DIR: &str = "Imported";
pub const FAILED_DIR: &str = "Couldn't import";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxSettings {
    pub enabled: bool,
    pub folder: String,
}

pub fn default_folder() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/Library/Mobile Documents/com~apple~CloudDocs/Hearken Inbox")
}

impl Default for InboxSettings {
    fn default() -> Self {
        Self { enabled: false, folder: default_folder() }
    }
}

fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(SETTINGS_FILE))
}

pub fn load_settings<R: Runtime>(app: &AppHandle<R>) -> InboxSettings {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

/// "Budget_review-2026 10 14.m4a" -> "Budget review 2026 10 14"
pub fn title_from_file(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported recording");
    let t = stem.replace(['_', '-'], " ");
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() { "Imported recording".into() } else { t }
}

/// Moves `file` into `folder/sub/`, never overwriting (adds " 2", " 3"...).
pub fn move_into(file: &Path, folder: &Path, sub: &str) -> std::io::Result<PathBuf> {
    let dir = folder.join(sub);
    std::fs::create_dir_all(&dir)?;
    let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.clone(), String::new()),
    };
    let mut target = dir.join(&name);
    let mut n = 2;
    while target.exists() {
        target = dir.join(format!("{stem} {n}{ext}"));
        n += 1;
    }
    std::fs::rename(file, &target)?;
    Ok(target)
}

/// Audio files ready to import: settled for SETTLE and the same size as last scan.
/// iCloud placeholders (".name.m4a.icloud") are asked to download and picked up later.
pub fn ready_files(folder: &Path, last_sizes: &mut HashMap<PathBuf, u64>, now: SystemTime) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut ready = Vec::new();
    let mut seen = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            if name.ends_with(".icloud") {
                let _ = std::process::Command::new("/usr/bin/brctl").arg("download").arg(&path).output();
            }
            continue;
        }
        if !path.is_file() || !is_audio(&path) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let size = meta.len();
        let settled = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age >= SETTLE);
        if size > 0 && settled && last_sizes.get(&path) == Some(&size) {
            ready.push(path.clone());
        }
        seen.insert(path, size);
    }
    *last_sizes = seen;
    ready.sort();
    ready
}

async fn import_one<R: Runtime>(app: &AppHandle<R>, folder: &Path, file: &Path) {
    let Some(state) = app.try_state::<AppState>() else { return };
    let config = SettingsRepository::get_transcript_config(state.db_manager.pool()).await.ok().flatten();
    let (provider, model) = match config {
        Some(c) => (Some(c.provider), Some(c.model)),
        None => (None, None),
    };
    let title = title_from_file(file);
    log::info!("Inbox: importing {}", file.display());
    let result = crate::audio::import::start_import(app.clone(), file.to_string_lossy().into_owned(), title.clone(), None, model, provider).await;
    match result {
        Ok(imported) => {
            let moved = move_into(file, folder, IMPORTED_DIR);
            if let Err(e) = &moved {
                log::warn!("Inbox: imported {} but couldn't move it: {}", file.display(), e);
            }
            let _ = app
                .notification()
                .builder()
                .title("Imported from your Hearken Inbox")
                .body(format!("{} ({} min). Open Hearken to summarise it.", imported.title, (imported.duration_seconds / 60.0).round()))
                .show();
        }
        Err(e) => {
            log::warn!("Inbox: couldn't import {}: {}", file.display(), e);
            let _ = move_into(file, folder, FAILED_DIR);
            let _ = app
                .notification()
                .builder()
                .title("Couldn't import a recording")
                .body(format!("{}: {}. It's in the \"{}\" folder.", title, e, FAILED_DIR))
                .show();
        }
    }
}

pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let mut last_sizes: HashMap<PathBuf, u64> = HashMap::new();
        loop {
            tokio::time::sleep(SCAN_EVERY).await;
            let settings = load_settings(&app);
            if !settings.enabled
                || crate::audio::recording_commands::current_recording_seconds().is_some()
                || crate::audio::import::is_import_in_progress()
                || app.try_state::<AppState>().is_none()
            {
                continue;
            }
            let folder = PathBuf::from(&settings.folder);
            let ready = ready_files(&folder, &mut last_sizes, SystemTime::now());
            // One at a time: an import transcribes the whole file.
            if let Some(file) = ready.first() {
                import_one(&app, &folder, file).await;
                last_sizes.remove(file);
            }
        }
    });
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxStatus {
    pub settings: InboxSettings,
    pub folder_exists: bool,
    pub waiting: usize,
    pub imported: usize,
    pub failed: usize,
}

fn count_audio(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|e| e.flatten().filter(|f| is_audio(&f.path())).count()).unwrap_or(0)
}

#[tauri::command]
pub fn api_inbox_status<R: Runtime>(app: AppHandle<R>) -> InboxStatus {
    let settings = load_settings(&app);
    let folder = PathBuf::from(&settings.folder);
    InboxStatus {
        folder_exists: folder.is_dir(),
        waiting: count_audio(&folder),
        imported: count_audio(&folder.join(IMPORTED_DIR)),
        failed: count_audio(&folder.join(FAILED_DIR)),
        settings,
    }
}

/// Saves the settings; turning it on creates the folder if needed.
#[tauri::command]
pub fn api_set_inbox<R: Runtime>(app: AppHandle<R>, settings: InboxSettings) -> Result<InboxStatus, String> {
    let folder = settings.folder.trim();
    if folder.is_empty() {
        return Err("Choose a folder".into());
    }
    if settings.enabled {
        std::fs::create_dir_all(folder).map_err(|e| format!("Couldn't create {}: {}", folder, e))?;
    }
    let path = settings_path(&app).ok_or("No app data folder")?;
    let clean = InboxSettings { enabled: settings.enabled, folder: folder.to_string() };
    std::fs::write(path, serde_json::to_string_pretty(&clean).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(api_inbox_status(app))
}

#[tauri::command]
pub fn api_reveal_inbox<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let folder = load_settings(&app).folder;
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    std::process::Command::new("/usr/bin/open").arg(&folder).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_come_from_file_names() {
        assert_eq!(title_from_file(Path::new("/x/Budget_review-2026 10 14.m4a")), "Budget review 2026 10 14");
        assert_eq!(title_from_file(Path::new("/x/___.m4a")), "Imported recording");
    }

    #[test]
    fn waits_for_files_to_settle_and_never_overwrites_when_moving() {
        let dir = std::env::temp_dir().join(format!("hearken-inbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let memo = dir.join("Lecture 3.m4a");
        std::fs::write(&memo, b"audio").unwrap();
        std::fs::write(dir.join("notes.txt"), b"not audio").unwrap();
        std::fs::write(dir.join(".Other.m4a.icloud"), b"placeholder").unwrap();

        let later = SystemTime::now() + Duration::from_secs(60);
        let mut sizes = HashMap::new();
        assert!(ready_files(&dir, &mut sizes, later).is_empty(), "first sight: not ready yet");
        assert_eq!(ready_files(&dir, &mut sizes, later), vec![memo.clone()], "same size next scan: ready");
        assert!(ready_files(&dir, &mut HashMap::new(), SystemTime::now()).is_empty(), "just modified: wait");

        let first = move_into(&memo, &dir, IMPORTED_DIR).unwrap();
        std::fs::write(&memo, b"another").unwrap();
        let second = move_into(&memo, &dir, IMPORTED_DIR).unwrap();
        assert_eq!(first.file_name().unwrap(), "Lecture 3.m4a");
        assert_eq!(second.file_name().unwrap(), "Lecture 3 2.m4a");
        assert!(first.exists() && second.exists(), "nothing deleted or overwritten");
        std::fs::remove_dir_all(dir).ok();
    }
}
