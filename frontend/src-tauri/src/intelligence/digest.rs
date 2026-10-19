//! The weekly digest: what happened this week (meetings, decisions, actions created and
//! finished) and what needs attention (overdue, due in the next 7 days), by project.
//! Built straight from the database, no AI involved, so it is exact. A notification
//! points to it on Friday afternoons, once per week.

use crate::state::AppState;
use chrono::{Datelike, Duration, Local, NaiveDate, Timelike};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Runtime};
use tauri_plugin_notification::NotificationExt;

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DigestMeeting {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub project_name: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DigestItem {
    pub id: String,
    pub meeting_id: String,
    pub meeting_title: String,
    pub title: String,
    pub owner: Option<String>,
    pub due_at: Option<String>,
    pub status: String,
    pub project_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWeek {
    pub project_name: String,
    pub meetings: usize,
    pub decisions: usize,
    pub open_actions: usize,
    pub overdue: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekDigest {
    /// Monday, YYYY-MM-DD
    pub week_start: String,
    /// Sunday, YYYY-MM-DD
    pub week_end: String,
    pub meetings: Vec<DigestMeeting>,
    pub decisions: Vec<DigestItem>,
    pub actions_created: Vec<DigestItem>,
    pub actions_done: Vec<DigestItem>,
    pub overdue: Vec<DigestItem>,
    pub due_soon: Vec<DigestItem>,
    pub by_project: Vec<ProjectWeek>,
}

const ITEM_SELECT: &str = "SELECT i.id, i.meeting_id, m.title AS meeting_title, i.title, i.owner, i.due_at, i.status,
        (SELECT p.name FROM meeting_projects mp JOIN projects p ON p.id = mp.project_id
          WHERE mp.meeting_id = m.id AND mp.is_primary = 1) AS project_name
     FROM meeting_items i JOIN meetings m ON m.id = i.meeting_id";

fn monday_of(day: NaiveDate) -> NaiveDate {
    day - Duration::days(day.weekday().num_days_from_monday() as i64)
}

pub async fn week_digest(pool: &SqlitePool, today: NaiveDate, weeks_back: i64) -> Result<WeekDigest, String> {
    let start = monday_of(today) - Duration::weeks(weeks_back);
    let end = start + Duration::days(6);
    let (s, e) = (start.format("%Y-%m-%d").to_string(), end.format("%Y-%m-%d").to_string());
    let today_s = today.format("%Y-%m-%d").to_string();
    let soon = (today + Duration::days(7)).format("%Y-%m-%d").to_string();
    let in_week = |col: &str| format!("substr({col}, 1, 10) BETWEEN ?1 AND ?2");

    let meetings = sqlx::query_as::<_, DigestMeeting>(&format!(
        "SELECT m.id, m.title, m.created_at,
            (SELECT p.name FROM meeting_projects mp JOIN projects p ON p.id = mp.project_id
              WHERE mp.meeting_id = m.id AND mp.is_primary = 1) AS project_name
         FROM meetings m WHERE {} ORDER BY m.created_at",
        in_week("m.created_at")
    ))
    .bind(&s)
    .bind(&e)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let items = |filter: String, order: &'static str| {
        let (s, e) = (s.clone(), e.clone());
        let (today_s, soon) = (today_s.clone(), soon.clone());
        async move {
            sqlx::query_as::<_, DigestItem>(&format!("{ITEM_SELECT} WHERE {filter} ORDER BY {order}"))
                .bind(s)
                .bind(e)
                .bind(today_s)
                .bind(soon)
                .fetch_all(pool)
                .await
                .map_err(|e| e.to_string())
        }
    };
    let decisions = items(format!("i.kind = 'decision' AND i.status != 'dismissed' AND {}", in_week("m.created_at")), "m.created_at").await?;
    let actions_created = items(format!("i.kind = 'action' AND i.status != 'dismissed' AND {}", in_week("i.created_at")), "i.created_at").await?;
    let actions_done = items(format!("i.kind = 'action' AND i.status = 'done' AND {}", in_week("i.updated_at")), "i.updated_at").await?;
    let overdue = items("i.kind = 'action' AND i.status = 'open' AND i.due_at IS NOT NULL AND i.due_at < ?3".into(), "i.due_at").await?;
    let due_soon = items("i.kind = 'action' AND i.status = 'open' AND i.due_at >= ?3 AND i.due_at <= ?4".into(), "i.due_at").await?;

    let mut names: Vec<String> = meetings.iter().filter_map(|m| m.project_name.clone())
        .chain(overdue.iter().chain(due_soon.iter()).filter_map(|i| i.project_name.clone()))
        .collect();
    names.sort();
    names.dedup();
    let by_project = names
        .into_iter()
        .map(|name| ProjectWeek {
            meetings: meetings.iter().filter(|m| m.project_name.as_deref() == Some(&name)).count(),
            decisions: decisions.iter().filter(|d| d.project_name.as_deref() == Some(&name)).count(),
            open_actions: actions_created.iter().filter(|a| a.project_name.as_deref() == Some(&name) && a.status == "open").count(),
            overdue: overdue.iter().filter(|a| a.project_name.as_deref() == Some(&name)).count(),
            project_name: name,
        })
        .collect();

    Ok(WeekDigest { week_start: s, week_end: e, meetings, decisions, actions_created, actions_done, overdue, due_soon, by_project })
}

#[tauri::command]
pub async fn api_weekly_digest(state: tauri::State<'_, AppState>, weeks_back: Option<i64>) -> Result<WeekDigest, String> {
    week_digest(state.db_manager.pool(), Local::now().date_naive(), weeks_back.unwrap_or(0).clamp(0, 52)).await
}

/// Friday 16:00–20:00, once per ISO week: a notification with the week's numbers.
pub async fn maybe_notify<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool) {
    let now = Local::now();
    if now.weekday() != chrono::Weekday::Fri || !(16..20).contains(&now.hour()) {
        return;
    }
    let week = format!("{}-W{:02}", now.iso_week().year(), now.iso_week().week());
    let sent: Option<String> = sqlx::query_scalar("SELECT value FROM app_state WHERE key = 'digest_notified_week'")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    if sent.as_deref() == Some(week.as_str()) {
        return;
    }
    let Ok(d) = week_digest(pool, now.date_naive(), 0).await else { return };
    if d.meetings.is_empty() && d.overdue.is_empty() {
        return;
    }
    let mut body = format!("{} meeting(s), {} decision(s)", d.meetings.len(), d.decisions.len());
    if !d.overdue.is_empty() {
        body.push_str(&format!(", {} overdue action(s)", d.overdue.len()));
    }
    body.push_str(". Open Hearken → Weekly digest.");
    if app.notification().builder().title("Your week in Hearken").body(&body).show().is_ok() {
        let _ = sqlx::query("INSERT OR REPLACE INTO app_state (key, value) VALUES ('digest_notified_week', ?)")
            .bind(&week)
            .execute(pool)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn digest_covers_the_week_and_what_needs_attention() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name, created_at, updated_at) VALUES ('p1', 'Budget 2027', 'x', 'x')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES
            ('m1', 'This week', '2026-10-14T10:00:00Z', 'x'), ('m0', 'Last week', '2026-10-07T10:00:00Z', 'x')").execute(&pool).await.unwrap();
        crate::projects::set_meeting_projects(&pool, "m1", Some("p1"), &[]).await.unwrap();
        sqlx::query("INSERT INTO meeting_items (id, meeting_id, kind, title, due_at, status, source, evidence_verified, created_at, updated_at) VALUES
            ('d1', 'm1', 'decision', 'Pause the lease', NULL, 'open', 'ai', 1, '2026-10-14T11:00:00Z', '2026-10-14T11:00:00Z'),
            ('a1', 'm1', 'action', 'Send forecast', '2026-10-16', 'open', 'ai', 1, '2026-10-14T11:00:00Z', '2026-10-14T11:00:00Z'),
            ('a0', 'm0', 'action', 'Old task', '2026-10-09', 'open', 'ai', 1, '2026-10-07T11:00:00Z', '2026-10-07T11:00:00Z')")
            .execute(&pool).await.unwrap();
        let thursday = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let d = week_digest(&pool, thursday, 0).await.unwrap();
        assert_eq!((d.week_start.as_str(), d.week_end.as_str()), ("2026-10-12", "2026-10-18"));
        assert_eq!(d.meetings.len(), 1);
        assert_eq!(d.decisions[0].title, "Pause the lease");
        assert_eq!(d.overdue.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), vec!["Old task"]);
        assert_eq!(d.due_soon.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), vec!["Send forecast"]);
        assert_eq!(d.by_project.len(), 1);
        assert_eq!(d.by_project[0].project_name, "Budget 2027");
        let last = week_digest(&pool, thursday, 1).await.unwrap();
        assert_eq!(last.meetings[0].title, "Last week");
    }
}
