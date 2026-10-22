//! A meeting's date is when the recording started. The recorder writes that time to the
//! folder's metadata.json; meetings saved before this fix were dated when the recording
//! stopped, and are corrected once at startup.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::path::Path;

const FIXED_FLAG: &str = "meeting_start_times_fixed";

/// The recording's start time from `<folder>/metadata.json`, if it's there.
pub fn recording_started_at(folder: &Path) -> Option<DateTime<Utc>> {
    let text = std::fs::read_to_string(folder.join("metadata.json")).ok()?;
    let meta: serde_json::Value = serde_json::from_str(&text).ok()?;
    let started = meta.get("created_at")?.as_str()?;
    DateTime::parse_from_rfc3339(started).ok().map(|d| d.with_timezone(&Utc))
}

/// Once per database: date recorded meetings by their start. Returns how many changed.
pub async fn fix_existing(pool: &SqlitePool) -> Result<usize, sqlx::Error> {
    let done: Option<String> = sqlx::query_scalar("SELECT value FROM app_state WHERE key = ?")
        .bind(FIXED_FLAG)
        .fetch_optional(pool)
        .await?;
    if done.is_some() {
        return Ok(0);
    }
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, created_at, folder_path FROM meetings WHERE folder_path IS NOT NULL")
            .fetch_all(pool)
            .await?;
    let mut changed = 0;
    for (id, created_at, folder) in rows {
        let Some(started) = recording_started_at(Path::new(&folder)) else { continue };
        let saved = DateTime::parse_from_rfc3339(&created_at).ok().map(|d| d.with_timezone(&Utc));
        // Only move a date earlier, and only when it's clearly the stop time (an import's
        // metadata, or a meeting already dated by its start, is left alone).
        if saved.is_some_and(|s| s > started && s - started > chrono::Duration::seconds(60)) {
            sqlx::query("UPDATE meetings SET created_at = ? WHERE id = ?")
                .bind(started)
                .bind(&id)
                .execute(pool)
                .await?;
            changed += 1;
        }
    }
    sqlx::query("INSERT OR REPLACE INTO app_state (key, value) VALUES (?, ?)")
        .bind(FIXED_FLAG)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn meetings_are_dated_by_when_recording_started() {
        let dir = std::env::temp_dir().join(format!("hearken-start-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("metadata.json"), r#"{"created_at": "2026-10-22T09:22:50.804577+00:00"}"#).unwrap();

        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, folder_path) VALUES ('m1', 'A', '2026-10-22T10:10:11.866241+00:00', 'x', ?)")
            .bind(dir.to_string_lossy().as_ref())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, folder_path) VALUES ('m2', 'B', '2026-10-20T08:00:00+00:00', 'x', '/nowhere')")
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(fix_existing(&pool).await.unwrap(), 1);
        let created: String = sqlx::query_scalar("SELECT created_at FROM meetings WHERE id = 'm1'").fetch_one(&pool).await.unwrap();
        assert!(created.starts_with("2026-10-22T09:22:50") || created.starts_with("2026-10-22 09:22:50"), "{created}");
        let untouched: String = sqlx::query_scalar("SELECT created_at FROM meetings WHERE id = 'm2'").fetch_one(&pool).await.unwrap();
        assert_eq!(untouched, "2026-10-20T08:00:00+00:00");
        assert_eq!(fix_existing(&pool).await.unwrap(), 0, "runs once");
        std::fs::remove_dir_all(dir).ok();
    }
}
