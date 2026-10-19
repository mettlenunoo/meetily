//! People: everyone named as the owner of an action item, with what they've committed to.
//! Names are grouped ignoring case and spacing; "Me" (the user, from speaker labels) is
//! shown as "You". Names are never merged by guesswork ("Kofi" and "Kofi Mensah" stay
//! separate unless written the same way).

use crate::state::AppState;
use serde::Serialize;
use sqlx::SqlitePool;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub name: String,
    pub open: i64,
    pub overdue: i64,
    pub done: i64,
    pub meetings: i64,
    pub last_seen: Option<String>,
    pub projects: Vec<String>,
}

pub async fn list_people(pool: &SqlitePool, today: &str) -> Result<Vec<Person>, String> {
    let rows: Vec<(String, i64, i64, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT MIN(trim(i.owner)) AS name,
            SUM(i.status = 'open'),
            SUM(i.status = 'open' AND i.due_at IS NOT NULL AND i.due_at < ?1),
            SUM(i.status = 'done'),
            COUNT(DISTINCT i.meeting_id),
            MAX(m.created_at)
         FROM meeting_items i JOIN meetings m ON m.id = i.meeting_id
         WHERE i.kind = 'action' AND i.status != 'dismissed' AND trim(COALESCE(i.owner, '')) != ''
         GROUP BY lower(replace(trim(i.owner), '  ', ' '))
         ORDER BY SUM(i.status = 'open' AND i.due_at IS NOT NULL AND i.due_at < ?1) DESC, SUM(i.status = 'open') DESC, name",
    )
    .bind(today)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (name, open, overdue, done, meetings, last_seen) in rows {
        let projects: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT p.name FROM meeting_items i
             JOIN meeting_projects mp ON mp.meeting_id = i.meeting_id AND mp.is_primary = 1
             JOIN projects p ON p.id = mp.project_id
             WHERE i.kind = 'action' AND lower(trim(i.owner)) = lower(?1) ORDER BY p.name",
        )
        .bind(&name)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        out.push(Person { name, open, overdue, done, meetings, last_seen, projects });
    }
    Ok(out)
}

#[tauri::command]
pub async fn api_list_people(state: tauri::State<'_, AppState>) -> Result<Vec<Person>, String> {
    let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
    list_people(state.db_manager.pool(), &today).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn groups_owners_and_counts_overdue() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1', 'A', '2026-10-14T10:00:00Z', 'x'), ('m2', 'B', '2026-10-16T10:00:00Z', 'x')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name, created_at, updated_at) VALUES ('p1', 'Budget', 'x', 'x')").execute(&pool).await.unwrap();
        crate::projects::set_meeting_projects(&pool, "m1", Some("p1"), &[]).await.unwrap();
        sqlx::query("INSERT INTO meeting_items (id, meeting_id, kind, title, owner, due_at, status, source, evidence_verified, created_at, updated_at) VALUES
            ('1', 'm1', 'action', 'a', 'Kofi', '2026-01-01', 'open', 'ai', 1, 'x', 'x'),
            ('2', 'm2', 'action', 'b', ' kofi ', NULL, 'open', 'ai', 1, 'x', 'x'),
            ('3', 'm2', 'action', 'c', 'Kofi Mensah', NULL, 'done', 'ai', 1, 'x', 'x'),
            ('4', 'm2', 'action', 'd', NULL, NULL, 'open', 'ai', 1, 'x', 'x'),
            ('5', 'm2', 'decision', 'e', 'Ama', NULL, 'open', 'ai', 1, 'x', 'x')")
            .execute(&pool).await.unwrap();
        let people = list_people(&pool, "2026-10-19").await.unwrap();
        let names: Vec<&str> = people.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names.len(), 2, "Kofi (any case/spacing) and Kofi Mensah; no owner and decisions skipped: {names:?}");
        let kofi = &people[0];
        assert_eq!((kofi.open, kofi.overdue, kofi.meetings), (2, 1, 2));
        assert_eq!(kofi.projects, vec!["Budget"]);
        assert_eq!(people[1].name, "Kofi Mensah");
        assert_eq!(people[1].done, 1);
    }
}
