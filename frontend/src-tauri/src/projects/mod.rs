//! Projects: what a recording is about. A meeting has one main project and any number of
//! extra ones. Hearken suggests a main project after a recording (see `suggest`), but nothing
//! is filed until the user accepts. Courses are projects with kind 'course' (the UI shows
//! them in the Study area). Projects marked 'local' are hidden from hearken-mcp.

pub mod people;
pub mod reminders;
pub mod suggest;

use crate::state::AppState;
use crate::summary::meeting_items::TrackedItem;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

pub const KINDS: &[&str] = &["work", "client", "personal", "course"];

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
    pub people: Option<String>,
    pub keywords: Option<String>,
    pub slack_channels: Option<String>,
    pub status: String,
    pub privacy: String,
    pub color: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A project with the counts shown in lists.
#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub project: Project,
    pub meeting_count: i64,
    pub open_actions: i64,
    pub overdue_actions: i64,
    pub last_meeting_at: Option<String>,
}

/// Fields for creating or editing a project. On edit, omitted fields stay as they are
/// and an empty string clears an optional field.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInput {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub description: Option<String>,
    pub people: Option<String>,
    pub keywords: Option<String>,
    pub slack_channels: Option<String>,
    pub status: Option<String>,
    pub privacy: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMeeting {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub is_primary: bool,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetail {
    pub project: Project,
    pub meetings: Vec<ProjectMeeting>,
    /// Action items from every linked meeting (main or extra), open first.
    pub actions: Vec<TrackedItem>,
    pub decisions: Vec<TrackedItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSuggestion {
    pub project: Project,
    pub reason: Option<String>,
    /// "match" (names, keywords or people found) or "ai" (the summary model chose it)
    pub source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingProjects {
    pub primary: Option<Project>,
    pub extras: Vec<Project>,
    /// Only while pending and the meeting has no main project yet.
    pub suggestion: Option<ProjectSuggestion>,
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

/// Trims a comma-separated list and drops empty entries: " a, ,b " -> "a, b".
fn clean_list(v: &str) -> Option<String> {
    let parts: Vec<&str> = v.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn validate(input: &ProjectInput) -> Result<(), String> {
    if let Some(k) = &input.kind {
        if !KINDS.contains(&k.as_str()) {
            return Err(format!("Invalid kind: {}", k));
        }
    }
    if let Some(s) = &input.status {
        if !matches!(s.as_str(), "active" | "archived") {
            return Err(format!("Invalid status: {}", s));
        }
    }
    if let Some(p) = &input.privacy {
        if !matches!(p.as_str(), "claude" | "local") {
            return Err(format!("Invalid privacy setting: {}", p));
        }
    }
    Ok(())
}

/// True when the meeting is linked (as main or extra) to a local-only project. Errors count
/// as local-only, so a database problem can never cause a recording to be sent out.
pub async fn is_local_only(pool: &SqlitePool, meeting_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM meeting_projects mp JOIN projects p ON p.id = mp.project_id
         WHERE mp.meeting_id = ? AND p.privacy = 'local'",
    )
    .bind(meeting_id)
    .fetch_one(pool)
    .await
    .map(|n| n > 0)
    .unwrap_or(true)
}

pub async fn get_project(pool: &SqlitePool, id: &str) -> Result<Project, String> {
    sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Project not found: {}", id))
}

fn unique_name_error(e: sqlx::Error, name: &str) -> String {
    if e.to_string().contains("UNIQUE") {
        format!("A project called \"{}\" already exists", name)
    } else {
        e.to_string()
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Projects with counts, most recently used first. `kind` filters (e.g. "course" for the
/// Study area); `exclude_kind` hides one (e.g. "course" for the Projects list).
#[tauri::command]
pub async fn api_list_projects(
    state: tauri::State<'_, AppState>,
    include_archived: Option<bool>,
    kind: Option<String>,
    exclude_kind: Option<String>,
) -> Result<Vec<ProjectSummary>, String> {
    let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
    sqlx::query_as::<_, ProjectSummary>(
        "SELECT p.*,
            (SELECT COUNT(*) FROM meeting_projects mp WHERE mp.project_id = p.id) AS meeting_count,
            (SELECT COUNT(*) FROM meeting_items i JOIN meeting_projects mp ON mp.meeting_id = i.meeting_id
              WHERE mp.project_id = p.id AND i.kind = 'action' AND i.status = 'open') AS open_actions,
            (SELECT COUNT(*) FROM meeting_items i JOIN meeting_projects mp ON mp.meeting_id = i.meeting_id
              WHERE mp.project_id = p.id AND i.kind = 'action' AND i.status = 'open'
                AND i.due_at IS NOT NULL AND i.due_at < ?1) AS overdue_actions,
            (SELECT MAX(m.created_at) FROM meetings m JOIN meeting_projects mp ON mp.meeting_id = m.id
              WHERE mp.project_id = p.id) AS last_meeting_at
         FROM projects p
         WHERE (?2 OR p.status = 'active')
           AND (?3 IS NULL OR p.kind = ?3)
           AND (?4 IS NULL OR p.kind != ?4)
         ORDER BY p.status = 'archived', COALESCE(last_meeting_at, p.created_at) DESC",
    )
    .bind(today)
    .bind(include_archived.unwrap_or(false))
    .bind(kind)
    .bind(exclude_kind)
    .fetch_all(state.db_manager.pool())
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_create_project(state: tauri::State<'_, AppState>, input: ProjectInput) -> Result<Project, String> {
    validate(&input)?;
    let name = input.name.as_deref().map(str::trim).filter(|s| !s.is_empty()).ok_or("A project needs a name")?;
    let ts = now();
    let id = Uuid::new_v4().to_string();
    let opt = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let list = |v: &Option<String>| v.as_deref().and_then(clean_list);
    sqlx::query(
        "INSERT INTO projects (id, name, kind, description, people, keywords, slack_channels, status, privacy, color, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(input.kind.as_deref().unwrap_or("work"))
    .bind(opt(&input.description))
    .bind(list(&input.people))
    .bind(list(&input.keywords))
    .bind(list(&input.slack_channels))
    .bind(input.privacy.as_deref().unwrap_or("claude"))
    .bind(opt(&input.color))
    .bind(&ts)
    .bind(&ts)
    .execute(state.db_manager.pool())
    .await
    .map_err(|e| unique_name_error(e, name))?;
    get_project(state.db_manager.pool(), &id).await
}

#[tauri::command]
pub async fn api_update_project(
    state: tauri::State<'_, AppState>,
    id: String,
    input: ProjectInput,
) -> Result<Project, String> {
    validate(&input)?;
    let pool = state.db_manager.pool();
    let current = get_project(pool, &id).await?;
    let name = match input.name.as_deref().map(str::trim) {
        Some("") => return Err("A project needs a name".into()),
        Some(n) => n.to_string(),
        None => current.name.clone(),
    };
    // None keeps the current value; "" clears it.
    let text = |new: &Option<String>, old: &Option<String>| match new {
        None => old.clone(),
        Some(v) => Some(v.trim().to_string()).filter(|s| !s.is_empty()),
    };
    let list = |new: &Option<String>, old: &Option<String>| match new {
        None => old.clone(),
        Some(v) => clean_list(v),
    };
    sqlx::query(
        "UPDATE projects SET name = ?, kind = ?, description = ?, people = ?, keywords = ?, slack_channels = ?,
            status = ?, privacy = ?, color = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&name)
    .bind(input.kind.as_deref().unwrap_or(&current.kind))
    .bind(text(&input.description, &current.description))
    .bind(list(&input.people, &current.people))
    .bind(list(&input.keywords, &current.keywords))
    .bind(list(&input.slack_channels, &current.slack_channels))
    .bind(input.status.as_deref().unwrap_or(&current.status))
    .bind(input.privacy.as_deref().unwrap_or(&current.privacy))
    .bind(text(&input.color, &current.color))
    .bind(now())
    .bind(&id)
    .execute(pool)
    .await
    .map_err(|e| unique_name_error(e, &name))?;
    get_project(pool, &id).await
}

/// Deletes the project and its links. The meetings themselves are kept.
#[tauri::command]
pub async fn api_delete_project(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(&id)
        .execute(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn api_get_project(state: tauri::State<'_, AppState>, id: String) -> Result<ProjectDetail, String> {
    let pool = state.db_manager.pool();
    let project = get_project(pool, &id).await?;
    let meetings = sqlx::query_as::<_, ProjectMeeting>(
        "SELECT m.id, m.title, m.created_at, mp.is_primary = 1 AS is_primary,
            (SELECT MAX(audio_end_time) FROM transcripts t WHERE t.meeting_id = m.id) AS duration_seconds
         FROM meeting_projects mp JOIN meetings m ON m.id = mp.meeting_id
         WHERE mp.project_id = ? ORDER BY m.created_at DESC",
    )
    .bind(&id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let items = |kind: &'static str| {
        let id = id.clone();
        async move {
            sqlx::query_as::<_, TrackedItem>(
                "SELECT i.id, i.meeting_id, i.kind, i.title, i.description, i.owner, i.due_date, i.due_at, i.priority,
                        i.status, i.evidence_quote, i.evidence_start_time, i.evidence_verified, i.source, i.user_edited,
                        i.created_at, i.updated_at,
                        m.title AS meeting_title, m.created_at AS meeting_created_at,
                        p.id AS project_id, p.name AS project_name
                 FROM meeting_items i
                 JOIN meetings m ON m.id = i.meeting_id
                 JOIN meeting_projects mp ON mp.meeting_id = m.id AND mp.project_id = ?1
                 JOIN projects p ON p.id = mp.project_id
                 WHERE i.kind = ?2 AND i.status != 'dismissed'
                 ORDER BY i.status != 'open', i.due_at IS NULL, i.due_at, m.created_at DESC",
            )
            .bind(&id)
            .bind(kind)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())
        }
    };
    Ok(ProjectDetail { project, meetings, actions: items("action").await?, decisions: items("decision").await? })
}

pub async fn meeting_projects(pool: &SqlitePool, meeting_id: &str) -> Result<MeetingProjects, String> {
    let linked = sqlx::query_as::<_, (String, bool)>(
        "SELECT project_id, is_primary = 1 FROM meeting_projects WHERE meeting_id = ? ORDER BY is_primary DESC, created_at",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let mut primary = None;
    let mut extras = Vec::new();
    for (pid, is_primary) in linked {
        let p = get_project(pool, &pid).await?;
        if is_primary {
            primary = Some(p);
        } else {
            extras.push(p);
        }
    }
    let suggestion = if primary.is_none() {
        match sqlx::query_as::<_, (String, Option<String>, String)>(
            "SELECT s.project_id, s.reason, s.source FROM project_suggestions s
             JOIN projects p ON p.id = s.project_id
             WHERE s.meeting_id = ? AND s.status = 'pending' AND p.status = 'active'",
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        {
            Some((pid, reason, source)) => Some(ProjectSuggestion { project: get_project(pool, &pid).await?, reason, source }),
            None => None,
        }
    } else {
        None
    };
    Ok(MeetingProjects { primary, extras, suggestion })
}

#[tauri::command]
pub async fn api_get_meeting_projects(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingProjects, String> {
    meeting_projects(state.db_manager.pool(), &meeting_id).await
}

/// Replaces a meeting's projects. A pending suggestion counts as accepted if the user
/// picked the suggested project as the main one, and as rejected otherwise.
pub async fn set_meeting_projects(
    pool: &SqlitePool,
    meeting_id: &str,
    primary_id: Option<&str>,
    extra_ids: &[String],
) -> Result<(), String> {
    let ts = now();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM meeting_projects WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(p) = primary_id {
        sqlx::query("INSERT INTO meeting_projects (meeting_id, project_id, is_primary, created_at) VALUES (?, ?, 1, ?)")
            .bind(meeting_id)
            .bind(p)
            .bind(&ts)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    for extra in extra_ids.iter().filter(|e| Some(e.as_str()) != primary_id) {
        sqlx::query("INSERT OR IGNORE INTO meeting_projects (meeting_id, project_id, is_primary, created_at) VALUES (?, ?, 0, ?)")
            .bind(meeting_id)
            .bind(extra)
            .bind(&ts)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    sqlx::query(
        "UPDATE project_suggestions
         SET status = CASE WHEN project_id = ?2 THEN 'accepted' ELSE 'rejected' END
         WHERE meeting_id = ?1 AND status = 'pending' AND ?2 IS NOT NULL",
    )
    .bind(meeting_id)
    .bind(primary_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    // Filing a recording under a course makes it a lecture, unless the user chose another type.
    if let Some(p) = primary_id {
        sqlx::query(
            "UPDATE meetings SET recording_type = 'lecture'
             WHERE id = ?1 AND recording_type = 'meeting'
               AND EXISTS (SELECT 1 FROM projects WHERE id = ?2 AND kind = 'course')",
        )
        .bind(meeting_id)
        .bind(p)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_set_meeting_projects(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    primary_id: Option<String>,
    extra_ids: Option<Vec<String>>,
) -> Result<MeetingProjects, String> {
    let pool = state.db_manager.pool();
    set_meeting_projects(pool, &meeting_id, primary_id.as_deref(), &extra_ids.unwrap_or_default()).await?;
    meeting_projects(pool, &meeting_id).await
}

/// Accept (make it the main project, keeping any extras) or dismiss the suggestion.
#[tauri::command]
pub async fn api_resolve_project_suggestion(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    accept: bool,
) -> Result<MeetingProjects, String> {
    let pool = state.db_manager.pool();
    let current = meeting_projects(pool, &meeting_id).await?;
    match (accept, current.suggestion) {
        (true, Some(s)) => {
            let extras: Vec<String> = current.extras.iter().map(|p| p.id.clone()).collect();
            set_meeting_projects(pool, &meeting_id, Some(&s.project.id), &extras).await?;
        }
        _ => {
            sqlx::query("UPDATE project_suggestions SET status = 'rejected' WHERE meeting_id = ? AND status = 'pending'")
                .bind(&meeting_id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    meeting_projects(pool, &meeting_id).await
}

/// Looks for a project by names, keywords and people only (fast, no model). Used when a
/// meeting is opened without a project; the model-based pass runs after summaries.
#[tauri::command]
pub async fn api_suggest_project(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingProjects, String> {
    let pool = state.db_manager.pool();
    suggest::suggest_by_matching(pool, &meeting_id).await?;
    meeting_projects(pool, &meeting_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_are_cleaned() {
        assert_eq!(clean_list(" Kofi, ,Ama ,"), Some("Kofi, Ama".into()));
        assert_eq!(clean_list(" , "), None);
    }

    #[test]
    fn rejects_unknown_kinds_and_privacy() {
        assert!(validate(&ProjectInput { kind: Some("hobby".into()), ..Default::default() }).is_err());
        assert!(validate(&ProjectInput { privacy: Some("public".into()), ..Default::default() }).is_err());
        assert!(validate(&ProjectInput { kind: Some("course".into()), privacy: Some("local".into()), ..Default::default() }).is_ok());
    }

    /// Real migrations on an in-memory database.
    async fn db() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1', 'IFDC Geoportal weekly', '2026-10-14T10:00:00Z', 'x')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time) VALUES
            ('t1', 'm1', 'Kofi showed the new GIS map layers.', 'x', 5.0)")
            .execute(&pool).await.unwrap();
        for (id, name, kw, people) in [("p1", "IFDC Geoportal", "GIS, map layers", "Kofi Mensah"), ("p2", "Surveyor POS", "receipts", ""), ("p3", "Wine2U", "", "")] {
            sqlx::query("INSERT INTO projects (id, name, keywords, people, created_at, updated_at) VALUES (?, ?, ?, ?, 'x', 'x')")
                .bind(id).bind(name).bind(kw).bind(people)
                .execute(&pool).await.unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn suggestion_waits_for_the_user_then_links() {
        let pool = db().await;
        assert!(suggest::suggest_by_matching(&pool, "m1").await.unwrap());
        let mp = meeting_projects(&pool, "m1").await.unwrap();
        assert!(mp.primary.is_none(), "nothing is linked until accepted");
        let s = mp.suggestion.expect("a pending suggestion");
        assert_eq!(s.project.id, "p1");
        assert_eq!(s.source, "match");
        assert!(!suggest::suggest_by_matching(&pool, "m1").await.unwrap(), "asked only once");

        set_meeting_projects(&pool, "m1", Some("p1"), &["p2".into(), "p1".into()]).await.unwrap();
        let mp = meeting_projects(&pool, "m1").await.unwrap();
        assert_eq!(mp.primary.unwrap().id, "p1");
        assert_eq!(mp.extras.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["p2"], "main project not duplicated as extra");
        assert!(mp.suggestion.is_none());
        let status: String = sqlx::query_scalar("SELECT status FROM project_suggestions WHERE meeting_id = 'm1'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(status, "accepted");
    }

    #[tokio::test]
    async fn picking_another_project_rejects_the_suggestion() {
        let pool = db().await;
        suggest::suggest_by_matching(&pool, "m1").await.unwrap();
        set_meeting_projects(&pool, "m1", Some("p3"), &[]).await.unwrap();
        let status: String = sqlx::query_scalar("SELECT status FROM project_suggestions WHERE meeting_id = 'm1'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(status, "rejected");
    }

    #[tokio::test]
    async fn only_one_main_project_and_deleting_a_project_keeps_meetings() {
        let pool = db().await;
        set_meeting_projects(&pool, "m1", Some("p1"), &[]).await.unwrap();
        let second_primary = sqlx::query("INSERT INTO meeting_projects (meeting_id, project_id, is_primary, created_at) VALUES ('m1', 'p2', 1, 'x')")
            .execute(&pool).await;
        assert!(second_primary.is_err(), "unique index allows one main project");

        sqlx::query("DELETE FROM projects WHERE id = 'p1'").execute(&pool).await.unwrap();
        let meetings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meetings").fetch_one(&pool).await.unwrap();
        let links: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meeting_projects").fetch_one(&pool).await.unwrap();
        assert_eq!((meetings, links), (1, 0));
    }

    #[tokio::test]
    async fn local_only_follows_any_link() {
        let pool = db().await;
        assert!(!is_local_only(&pool, "m1").await);
        sqlx::query("UPDATE projects SET privacy = 'local' WHERE id = 'p2'").execute(&pool).await.unwrap();
        set_meeting_projects(&pool, "m1", Some("p1"), &["p2".into()]).await.unwrap();
        assert!(is_local_only(&pool, "m1").await, "an extra local project is enough");
    }

    #[tokio::test]
    async fn filing_under_a_course_makes_it_a_lecture() {
        let pool = db().await;
        sqlx::query("UPDATE projects SET kind = 'course' WHERE id = 'p3'").execute(&pool).await.unwrap();
        set_meeting_projects(&pool, "m1", Some("p1"), &[]).await.unwrap();
        assert_eq!(crate::recordings::recording_type(&pool, "m1").await, "meeting");
        set_meeting_projects(&pool, "m1", Some("p3"), &[]).await.unwrap();
        assert_eq!(crate::recordings::recording_type(&pool, "m1").await, "lecture");
        crate::recordings::set_recording_type(&pool, "m1", "interview").await.unwrap();
        set_meeting_projects(&pool, "m1", Some("p3"), &[]).await.unwrap();
        assert_eq!(crate::recordings::recording_type(&pool, "m1").await, "interview", "a chosen type is kept");
    }

    #[tokio::test]
    async fn names_are_unique_ignoring_case() {
        let pool = db().await;
        let dup = sqlx::query("INSERT INTO projects (id, name, created_at, updated_at) VALUES ('p9', 'ifdc geoportal', 'x', 'x')")
            .execute(&pool).await;
        assert!(dup.is_err());
    }
}
