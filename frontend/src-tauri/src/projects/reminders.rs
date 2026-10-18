//! Local reminders for open action items that are overdue, due today or due tomorrow.
//! One Mac notification at most per item per day, only during waking hours. Nothing
//! leaves the machine.

use crate::state::AppState;
use chrono::{Duration, Local, Timelike};
use log::{info, warn};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const FIRST_CHECK_AFTER: std::time::Duration = std::time::Duration::from_secs(90);
const WAKING_HOURS: std::ops::Range<u32> = 8..20;

pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_AFTER).await;
        loop {
            if let Err(e) = check_once(&app).await {
                warn!("Action item reminder check failed: {}", e);
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// "2 overdue, 1 due today" plus the first few titles.
fn compose(rows: &[(String, String, String)], today: &str) -> (String, String) {
    let overdue = rows.iter().filter(|(_, _, due)| due.as_str() < today).count();
    let due_today = rows.iter().filter(|(_, _, due)| due == today).count();
    let tomorrow = rows.len() - overdue - due_today;
    let mut parts = Vec::new();
    if overdue > 0 { parts.push(format!("{} overdue", overdue)); }
    if due_today > 0 { parts.push(format!("{} due today", due_today)); }
    if tomorrow > 0 { parts.push(format!("{} due tomorrow", tomorrow)); }
    let title = format!("Action items: {}", parts.join(", "));
    let mut body: Vec<String> = rows.iter().take(3).map(|(_, t, _)| format!("• {}", t)).collect();
    if rows.len() > 3 {
        body.push(format!("and {} more", rows.len() - 3));
    }
    (title, body.join("\n"))
}

async fn check_once<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let now = Local::now();
    if !WAKING_HOURS.contains(&now.hour()) {
        return Ok(());
    }
    // The database isn't ready until onboarding has finished.
    let Some(state) = app.try_state::<AppState>() else { return Ok(()) };
    let pool = state.db_manager.pool();
    let today = now.date_naive().format("%Y-%m-%d").to_string();
    let tomorrow = (now.date_naive() + Duration::days(1)).format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, title, due_at FROM meeting_items
         WHERE kind = 'action' AND status = 'open' AND due_at IS NOT NULL AND due_at <= ?1
           AND (reminded_on IS NULL OR reminded_on < ?2)
         ORDER BY due_at",
    )
    .bind(&tomorrow)
    .bind(&today)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok(());
    }

    let (title, body) = compose(&rows, &today);
    app.notification().builder().title(&title).body(&body).show().map_err(|e| e.to_string())?;
    info!("Reminded about {} action item(s)", rows.len());

    for (id, _, _) in &rows {
        sqlx::query("UPDATE meeting_items SET reminded_on = ? WHERE id = ?")
            .bind(&today)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_a_short_summary() {
        let rows = vec![
            ("1".into(), "Send forecast".into(), "2026-10-12".into()),
            ("2".into(), "Book venue".into(), "2026-10-14".into()),
            ("3".into(), "Call bank".into(), "2026-10-15".into()),
            ("4".into(), "Pay invoice".into(), "2026-10-15".into()),
        ];
        let (title, body) = compose(&rows, "2026-10-14");
        assert_eq!(title, "Action items: 1 overdue, 1 due today, 2 due tomorrow");
        assert!(body.starts_with("• Send forecast"));
        assert!(body.ends_with("and 1 more"));
    }
}
