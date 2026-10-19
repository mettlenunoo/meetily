//! Calendar-aware recording, from the Mac's own Calendar (EventKit; it includes synced
//! Google and Exchange calendars). Read only, local only, and off until the user turns it
//! on and macOS grants access. When on:
//! - a recording made during a calendar event takes the event's title (if it still has the
//!   default "Meeting dd_mm_yy..." name) and remembers who was invited;
//! - the invitees and title help suggest the project;
//! - a notification reminds you to record when an event with other people starts.

use crate::state::AppState;
use chrono::{DateTime, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

const SETTINGS_FILE: &str = "calendar.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSettings {
    pub enabled: bool,
    /// Remind to record when an event with other people starts
    pub reminders: bool,
}

impl Default for CalendarSettings {
    fn default() -> Self {
        Self { enabled: false, reminders: true }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalEvent {
    pub id: String,
    pub title: String,
    pub calendar: Option<String>,
    /// Unix seconds
    pub start: f64,
    pub end: f64,
    pub attendees: Vec<String>,
    pub all_day: bool,
}

fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(SETTINGS_FILE))
}

pub fn load_settings<R: Runtime>(app: &AppHandle<R>) -> CalendarSettings {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
mod mac {
    use super::CalEvent;
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2::AnyThread;
    use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore};
    use objc2_foundation::{NSDate, NSError};

    pub fn status() -> &'static str {
        let s = unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) };
        match s {
            EKAuthorizationStatus::FullAccess => "granted",
            EKAuthorizationStatus::Denied | EKAuthorizationStatus::Restricted => "denied",
            EKAuthorizationStatus::WriteOnly => "write_only",
            _ => "not_asked",
        }
    }

    /// Shows the macOS permission prompt (first time only). Blocks until answered.
    pub fn request_access() -> bool {
        let store = unsafe { EKEventStore::init(EKEventStore::alloc()) };
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        let block = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
            let _ = tx.send(granted.as_bool());
        });
        unsafe { store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&block)) };
        rx.recv_timeout(std::time::Duration::from_secs(300)).unwrap_or(false)
    }

    /// Events overlapping [from, to] (Unix seconds), from every calendar.
    pub fn events(from: f64, to: f64) -> Vec<CalEvent> {
        if status() != "granted" {
            return Vec::new();
        }
        unsafe {
            let store = EKEventStore::init(EKEventStore::alloc());
            let start = NSDate::dateWithTimeIntervalSince1970(from);
            let end = NSDate::dateWithTimeIntervalSince1970(to);
            let predicate = store.predicateForEventsWithStartDate_endDate_calendars(&start, &end, None);
            let found = store.eventsMatchingPredicate(&predicate);
            found
                .iter()
                .map(|e| CalEvent {
                    id: e.eventIdentifier().map(|s| s.to_string()).unwrap_or_default(),
                    title: e.title().to_string(),
                    calendar: e.calendar().map(|c| c.title().to_string()),
                    start: e.startDate().timeIntervalSince1970(),
                    end: e.endDate().timeIntervalSince1970(),
                    attendees: e
                        .attendees()
                        .map(|list| list.iter().filter_map(|p| p.name().map(|n| n.to_string())).collect())
                        .unwrap_or_default(),
                    all_day: e.isAllDay(),
                })
                .collect()
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod mac {
    use super::CalEvent;
    pub fn status() -> &'static str { "unsupported" }
    pub fn request_access() -> bool { false }
    pub fn events(_from: f64, _to: f64) -> Vec<CalEvent> { Vec::new() }
}

/// The timed event that overlaps [start, end] the most (all-day events don't count).
pub fn best_overlap(events: &[CalEvent], start: f64, end: f64) -> Option<&CalEvent> {
    events
        .iter()
        .filter(|e| !e.all_day)
        .map(|e| (e, e.end.min(end) - e.start.max(start)))
        .filter(|(_, overlap)| *overlap > 60.0)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(e, _)| e)
}

/// Hearken's default name, e.g. "Meeting 18_10_26_21_47_23".
pub fn is_default_title(title: &str) -> bool {
    let rest = title.strip_prefix("Meeting ").unwrap_or("");
    rest.len() >= 8 && rest.chars().all(|c| c.is_ascii_digit() || c == '_' || c == '-' || c == ' ')
}

fn iso(secs: f64) -> String {
    Utc.timestamp_opt(secs as i64, 0).single().map(|d| d.to_rfc3339()).unwrap_or_default()
}

/// After a recording is saved: link it to the calendar event it happened during.
pub async fn attach_event<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool, meeting_id: &str) -> Result<Option<String>, String> {
    if !load_settings(app).enabled {
        return Ok(None);
    }
    let (title, created): (String, String) = sqlx::query_as("SELECT title, created_at FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Meeting not found")?;
    let length: Option<f64> = sqlx::query_scalar("SELECT MAX(audio_end_time) FROM transcripts WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    let saved_at = DateTime::parse_from_rfc3339(&created).map(|d| d.timestamp() as f64).unwrap_or_else(|_| Utc::now().timestamp() as f64);
    let (start, end) = (saved_at - length.unwrap_or(600.0), saved_at);
    let events = tokio::task::spawn_blocking(move || mac::events(start - 3600.0, end + 3600.0)).await.map_err(|e| e.to_string())?;
    let Some(event) = best_overlap(&events, start, end) else { return Ok(None) };

    sqlx::query(
        "INSERT OR REPLACE INTO meeting_calendar (meeting_id, event_title, calendar_name, starts_at, ends_at, attendees) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(meeting_id)
    .bind(&event.title)
    .bind(&event.calendar)
    .bind(iso(event.start))
    .bind(iso(event.end))
    .bind((!event.attendees.is_empty()).then(|| event.attendees.join(", ")))
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    if is_default_title(&title) && !event.title.trim().is_empty() {
        sqlx::query("UPDATE meetings SET title = ? WHERE id = ?")
            .bind(event.title.trim())
            .bind(meeting_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    log::info!("Linked {} to calendar event \"{}\" ({} invitees)", meeting_id, event.title, event.attendees.len());
    Ok(Some(event.title.clone()))
}

/// Reminds to record when an event with other people starts (once per event).
pub fn start_reminders<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let settings = load_settings(&app);
            if !settings.enabled || !settings.reminders || crate::audio::recording_commands::current_recording_seconds().is_some() {
                continue;
            }
            let Some(state) = app.try_state::<AppState>() else { continue };
            let pool = state.db_manager.pool().clone();
            let now = Utc::now().timestamp() as f64;
            let Ok(events) = tokio::task::spawn_blocking(move || mac::events(now - 120.0, now + 120.0)).await else { continue };
            for e in events.iter().filter(|e| !e.all_day && !e.attendees.is_empty() && (e.start - now).abs() <= 90.0) {
                let key = format!("cal_reminded:{}:{}", e.id, e.start as i64);
                let seen: Option<String> = sqlx::query_scalar("SELECT value FROM app_state WHERE key = ?")
                    .bind(&key)
                    .fetch_optional(&pool)
                    .await
                    .ok()
                    .flatten();
                if seen.is_some() {
                    continue;
                }
                let starts = Local.timestamp_opt(e.start as i64, 0).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default();
                let _ = app
                    .notification()
                    .builder()
                    .title(format!("{} starts {}", e.title, starts))
                    .body("Record it with Hearken? Press Start Recording (and tell people you're recording).")
                    .show();
                let _ = sqlx::query("INSERT OR REPLACE INTO app_state (key, value) VALUES (?, ?)")
                    .bind(&key)
                    .bind(Utc::now().to_rfc3339())
                    .execute(&pool)
                    .await;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    pub settings: CalendarSettings,
    /// "granted", "denied", "not_asked", "write_only" or "unsupported"
    pub access: String,
}

#[tauri::command]
pub fn api_calendar_status<R: Runtime>(app: AppHandle<R>) -> CalendarStatus {
    CalendarStatus { settings: load_settings(&app), access: mac::status().into() }
}

/// Saves the settings; turning it on asks macOS for calendar access the first time.
#[tauri::command]
pub async fn api_set_calendar<R: Runtime>(app: AppHandle<R>, settings: CalendarSettings) -> Result<CalendarStatus, String> {
    if settings.enabled && mac::status() != "granted" {
        let granted = tokio::task::spawn_blocking(mac::request_access).await.map_err(|e| e.to_string())?;
        if !granted {
            return Err("Calendar access wasn't granted. Allow Hearken in System Settings → Privacy & Security → Calendars.".into());
        }
    }
    let path = settings_path(&app).ok_or("No app data folder")?;
    std::fs::write(path, serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(api_calendar_status(app))
}

/// Today's remaining events, for the home screen.
#[tauri::command]
pub async fn api_upcoming_events<R: Runtime>(app: AppHandle<R>, hours: Option<f64>) -> Result<Vec<CalEvent>, String> {
    if !load_settings(&app).enabled {
        return Ok(Vec::new());
    }
    let now = Utc::now().timestamp() as f64;
    let span = hours.unwrap_or(12.0).clamp(1.0, 72.0) * 3600.0;
    let mut events = tokio::task::spawn_blocking(move || mac::events(now - 1800.0, now + span)).await.map_err(|e| e.to_string())?;
    events.retain(|e| !e.all_day && e.end > now);
    events.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    Ok(events)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MeetingEvent {
    pub event_title: String,
    pub calendar_name: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
    pub attendees: Option<String>,
}

#[tauri::command]
pub async fn api_meeting_event(state: tauri::State<'_, AppState>, meeting_id: String) -> Result<Option<MeetingEvent>, String> {
    sqlx::query_as::<_, MeetingEvent>("SELECT event_title, calendar_name, starts_at, ends_at, attendees FROM meeting_calendar WHERE meeting_id = ?")
        .bind(&meeting_id)
        .fetch_optional(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(title: &str, start: f64, end: f64, all_day: bool) -> CalEvent {
        CalEvent { id: title.into(), title: title.into(), calendar: None, start, end, attendees: vec![], all_day }
    }

    #[test]
    fn picks_the_event_the_recording_overlaps_most() {
        let events = vec![ev("Standup", 0.0, 900.0, false), ev("Budget review", 900.0, 4500.0, false), ev("Holiday", -1e6, 1e6, true)];
        assert_eq!(best_overlap(&events, 600.0, 3000.0).unwrap().title, "Budget review");
        assert!(best_overlap(&events, 5000.0, 6000.0).is_none(), "all-day events don't count");
        assert!(best_overlap(&[ev("Tiny", 0.0, 30.0, false)], 0.0, 600.0).is_none(), "a few seconds of overlap isn't a match");
    }

    #[test]
    fn only_default_names_are_replaced() {
        assert!(is_default_title("Meeting 18_10_26_21_47_23"));
        assert!(!is_default_title("Budget review"));
        assert!(!is_default_title("Meeting with Kofi"));
    }

    #[test]
    fn reads_the_real_calendar_when_allowed() {
        // Only does anything on a Mac where Hearken has calendar access.
        if mac::status() == "granted" {
            let now = Utc::now().timestamp() as f64;
            let _ = mac::events(now - 86400.0, now + 86400.0);
        }
    }
}
