//! Backups: is this data protected? Checks Time Machine, offers a one-click copy of the
//! database (a consistent snapshot with SQLite's VACUUM INTO, safe while Hearken runs),
//! shows where recordings live, and reminds at most monthly when nothing protects them.

use crate::state::AppState;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

const REMIND_EVERY_DAYS: i64 = 30;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    pub time_machine_configured: bool,
    pub time_machine_latest: Option<String>,
    pub last_manual_backup: Option<String>,
    pub database_path: Option<String>,
    pub recordings_folder: Option<String>,
    pub protected: bool,
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// "tmutil latestbackup" prints a path ending in a date like 2026-10-18-221530.
pub fn parse_tm_date(path: &str) -> Option<String> {
    let name = path.trim().trim_end_matches('/').rsplit('/').next()?;
    let stamp = name.split('.').next()?;
    let dt = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d-%H%M%S").ok()?;
    Some(dt.format("%Y-%m-%d %H:%M").to_string())
}

async fn app_state_value(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM app_state WHERE key = ?").bind(key).fetch_optional(pool).await.ok().flatten()
}

async fn status<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool) -> BackupStatus {
    let destinations = run("/usr/bin/tmutil", &["destinationinfo"]).unwrap_or_default();
    let configured = destinations.contains("Name") || destinations.contains("ID");
    let latest = if configured { run("/usr/bin/tmutil", &["latestbackup"]).and_then(|p| parse_tm_date(&p)) } else { None };
    let manual = app_state_value(pool, "last_manual_backup").await;
    let db = app.path().app_data_dir().ok().map(|d| d.join("meeting_minutes.sqlite").to_string_lossy().into_owned());
    let recordings = crate::audio::recording_preferences::get_default_recordings_folder().to_string_lossy().into_owned();
    let recent_manual = manual
        .as_deref()
        .and_then(|m| DateTime::parse_from_rfc3339(m).ok())
        .is_some_and(|d| Utc::now() - d.with_timezone(&Utc) < Duration::days(REMIND_EVERY_DAYS));
    BackupStatus {
        protected: configured || recent_manual,
        time_machine_configured: configured,
        time_machine_latest: latest,
        last_manual_backup: manual,
        database_path: db,
        recordings_folder: Some(recordings),
    }
}

#[tauri::command]
pub async fn api_backup_status<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>) -> Result<BackupStatus, String> {
    Ok(status(&app, state.db_manager.pool()).await)
}

/// Copies the database to a folder the user picks (e.g. an external drive). Returns the file.
#[tauri::command]
pub async fn api_backup_database<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let app2 = app.clone();
    let picked = tokio::task::spawn_blocking(move || app2.dialog().file().set_title("Choose where to save the backup").blocking_pick_folder())
        .await
        .map_err(|e| e.to_string())?;
    let Some(folder) = picked else { return Ok(None) };
    let folder: PathBuf = folder.into_path().map_err(|e| e.to_string())?;
    let target = folder.join(format!("Hearken backup {}.sqlite", chrono::Local::now().format("%Y-%m-%d %H%M")));
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }
    let pool = state.db_manager.pool();
    sqlx::query("VACUUM INTO ?")
        .bind(target.to_string_lossy().as_ref())
        .execute(pool)
        .await
        .map_err(|e| format!("Backup failed: {}", e))?;
    // Never report a backup that isn't there.
    let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return Err(format!("The backup wasn't written to {}", target.display()));
    }
    sqlx::query("INSERT OR REPLACE INTO app_state (key, value) VALUES ('last_manual_backup', ?)")
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(target.to_string_lossy().into_owned()))
}

/// Once a day: if nothing protects the data, a reminder at most every 30 days.
pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(10 * 60)).await;
        loop {
            if let Some(state) = app.try_state::<AppState>() {
                let pool = state.db_manager.pool().clone();
                let s = status(&app, &pool).await;
                let last = app_state_value(&pool, "backup_reminded").await
                    .and_then(|d| DateTime::parse_from_rfc3339(&d).ok())
                    .map(|d| d.with_timezone(&Utc));
                let due = last.is_none_or(|l| Utc::now() - l > Duration::days(REMIND_EVERY_DAYS));
                if !s.protected && due {
                    let shown = app
                        .notification()
                        .builder()
                        .title("Your Hearken recordings aren't backed up")
                        .body("Turn on Time Machine, or save a copy in Hearken → Settings → Backups.")
                        .show()
                        .is_ok();
                    if shown {
                        let _ = sqlx::query("INSERT OR REPLACE INTO app_state (key, value) VALUES ('backup_reminded', ?)")
                            .bind(Utc::now().to_rfc3339())
                            .execute(&pool)
                            .await;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_time_machine_dates() {
        assert_eq!(
            parse_tm_date("/Volumes/Backups/Backups.backupdb/Mac/2026-10-18-221530/").as_deref(),
            Some("2026-10-18 22:15")
        );
        assert_eq!(parse_tm_date("/Volumes/.timemachine/X/2026-10-18-221530.backup").as_deref(), Some("2026-10-18 22:15"));
        assert_eq!(parse_tm_date("nonsense"), None);
    }

    #[tokio::test]
    async fn vacuum_into_makes_a_complete_copy() {
        // A file database like the app's (VACUUM INTO writes nothing from an in-memory one).
        let src = std::env::temp_dir().join(format!("hearken-backup-src-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&src);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&src).create_if_missing(true))
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1', 'Kept', 'x', 'x')").execute(&pool).await.unwrap();
        let target = std::env::temp_dir().join(format!("hearken-backup-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&target);
        sqlx::query("VACUUM INTO ?").bind(target.to_string_lossy().as_ref()).execute(&pool).await.unwrap();
        let copy = sqlx::sqlite::SqlitePoolOptions::new().connect(&format!("sqlite://{}", target.display())).await.unwrap();
        let title: String = sqlx::query_scalar("SELECT title FROM meetings").fetch_one(&copy).await.unwrap();
        assert_eq!(title, "Kept");
        copy.close().await;
        std::fs::remove_file(target).ok();
        std::fs::remove_file(src).ok();
    }
}
