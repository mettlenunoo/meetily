//! A short, running "where things stand" for each project, rewritten after each summary
//! of one of its meetings (or on request). Built only from the project's own summaries
//! and recorded items, with the summary model; local-only projects only with a local model.

use super::current_model;
use crate::state::AppState;
use chrono::Utc;
use hearken_core::export::summary_markdown;
use sqlx::SqlitePool;
use tauri::{AppHandle, Runtime};

const MAX_MEETINGS: i64 = 8;
const MAX_SUMMARY_CHARS: usize = 1500;

async fn source(pool: &SqlitePool, project_id: &str) -> Result<Option<String>, String> {
    let project = crate::projects::get_project(pool, project_id).await?;
    let meetings: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT m.id, m.title, m.created_at FROM meeting_projects mp JOIN meetings m ON m.id = mp.meeting_id
         WHERE mp.project_id = ? ORDER BY m.created_at DESC LIMIT ?",
    )
    .bind(project_id)
    .bind(MAX_MEETINGS)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    if meetings.is_empty() {
        return Ok(None);
    }
    let today = chrono::Local::now().date_naive();
    let mut text = format!("Project: {}\nToday: {}\n", project.name, today);
    if let Some(d) = &project.description {
        text.push_str(&format!("About: {}\n", d));
    }
    for (id, title, created) in &meetings {
        let summary = sqlx::query_scalar::<_, Option<String>>("SELECT result FROM summary_processes WHERE meeting_id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
            .flatten()
            .and_then(|r| summary_markdown(&r))
            .map(|s| s.chars().take(MAX_SUMMARY_CHARS).collect::<String>())
            .unwrap_or_else(|| "(not summarised)".into());
        text.push_str(&format!("\n## {} ({})\n{}\n", title, &created[..created.len().min(10)], summary));
    }
    let open: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT i.title, i.owner, i.due_at FROM meeting_items i JOIN meeting_projects mp ON mp.meeting_id = i.meeting_id
         WHERE mp.project_id = ? AND i.kind = 'action' AND i.status = 'open' ORDER BY i.due_at IS NULL, i.due_at LIMIT 30",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    if !open.is_empty() {
        text.push_str("\nOpen action items:\n");
        let today = today.format("%Y-%m-%d").to_string();
        for (title, owner, due) in open {
            let overdue = due.as_deref().is_some_and(|d| d < today.as_str());
            text.push_str(&format!(
                "- {} (owner: {}, due: {}){}\n",
                title,
                owner.unwrap_or_else(|| "not stated".into()),
                due.unwrap_or_else(|| "not stated".into()),
                if overdue { " OVERDUE" } else { "" }
            ));
        }
    }
    Ok(Some(text))
}

pub async fn refresh<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool, project_id: &str) -> Result<Option<String>, String> {
    let project = crate::projects::get_project(pool, project_id).await?;
    let model = current_model(app, pool).await?;
    if project.privacy == "local" && !model.is_local() {
        return Err("This project is local-only and the summary model is a cloud model".into());
    }
    let Some(text) = source(pool, project_id).await? else { return Ok(None) };
    let raw = model
        .complete(
            "You keep a short status note for a project, using only the information given. You never guess.",
            &format!(
                "{text}\n\nWrite \"where things stand\" for this project as 4 to 7 short Markdown bullets: the current \
                 status, the latest decisions, what happens next, and any OVERDUE actions (with owner). Mention dates. \
                 No heading and no preamble."
            ),
        )
        .await?;
    let brief = crate::summary::processor::clean_llm_markdown_detailed(&raw).markdown.trim().to_string();
    if brief.is_empty() {
        return Err("The model returned an empty brief".into());
    }
    sqlx::query("UPDATE projects SET brief = ?, brief_updated_at = ? WHERE id = ?")
        .bind(&brief)
        .bind(Utc::now().to_rfc3339())
        .bind(project_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(brief))
}

/// After a meeting's summary: refresh the brief of its main project, if it has one.
pub async fn refresh_for_meeting<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool, meeting_id: &str) -> Result<(), String> {
    let project: Option<String> = sqlx::query_scalar("SELECT project_id FROM meeting_projects WHERE meeting_id = ? AND is_primary = 1")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(pid) = project {
        refresh(app, pool, &pid).await?;
    }
    Ok(())
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBrief {
    pub brief: Option<String>,
    pub updated_at: Option<String>,
}

#[tauri::command]
pub async fn api_get_project_brief(state: tauri::State<'_, AppState>, project_id: String) -> Result<ProjectBrief, String> {
    let (brief, updated_at): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT brief, brief_updated_at FROM projects WHERE id = ?")
            .bind(&project_id)
            .fetch_optional(state.db_manager.pool())
            .await
            .map_err(|e| e.to_string())?
            .ok_or("Project not found")?;
    Ok(ProjectBrief { brief, updated_at })
}

#[tauri::command]
pub async fn api_refresh_project_brief<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> Result<ProjectBrief, String> {
    let pool = state.db_manager.pool();
    refresh(&app, pool, &project_id).await?;
    api_get_project_brief(state, project_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn brief_source_has_recent_meetings_and_overdue_items() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name, created_at, updated_at) VALUES ('p1', 'Budget 2027', 'x', 'x')").execute(&pool).await.unwrap();
        assert!(source(&pool, "p1").await.unwrap().is_none(), "no meetings, no brief");
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1', 'Budget review', '2026-10-14T10:00:00Z', 'x')").execute(&pool).await.unwrap();
        crate::projects::set_meeting_projects(&pool, "m1", Some("p1"), &[]).await.unwrap();
        sqlx::query("INSERT INTO meeting_items (id, meeting_id, kind, title, owner, due_at, status, source, evidence_verified, created_at, updated_at)
                     VALUES ('i1', 'm1', 'action', 'Send forecast', 'Kofi', '2026-01-02', 'open', 'ai', 1, 'x', 'x')").execute(&pool).await.unwrap();
        let text = source(&pool, "p1").await.unwrap().unwrap();
        assert!(text.contains("## Budget review (2026-10-14)\n(not summarised)"));
        assert!(text.contains("- Send forecast (owner: Kofi, due: 2026-01-02) OVERDUE"));
    }
}
