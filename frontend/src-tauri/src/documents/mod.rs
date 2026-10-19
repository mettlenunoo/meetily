//! Documents: several write-ups per meeting (minutes, a formal report, lecture notes...)
//! next to its summary, and status reports for a whole project. Generated in the
//! background with the user's summary model (same chunking as summaries), editable,
//! and exportable as Markdown, Word (via macOS textutil) or PDF (via the print dialog).

use crate::database::repositories::setting::SettingsRepository;
use crate::state::AppState;
use crate::summary::llm_client::LLMProvider;
use crate::summary::processor::{clean_llm_markdown_detailed, generate_meeting_summary};
use chrono::Utc;
use hearken_core::export::summary_markdown;
use hearken_core::items::{load_timed_transcript, render_transcript};
use log::{error, info};
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

/// Templates offered for a project document; every other template is for one meeting.
pub const PROJECT_TEMPLATES: &[&str] = &["project_status_report"];

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: String,
    pub meeting_id: Option<String>,
    pub project_id: Option<String>,
    pub template_id: String,
    pub title: String,
    pub markdown: String,
    pub status: String,
    pub error: Option<String>,
    pub generated_by: String,
    pub user_edited: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentEvent {
    id: String,
    status: String,
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

/// The meeting title is shown separately, so a leading `# Title` line is dropped.
fn strip_title(markdown: &str) -> String {
    let t = markdown.trim_start();
    if t.starts_with("# ") {
        t.split_once('\n').map(|(_, rest)| rest.trim_start().to_string()).unwrap_or_default()
    } else {
        t.to_string()
    }
}

async fn get(pool: &SqlitePool, id: &str) -> Result<Document, String> {
    sqlx::query_as::<_, Document>("SELECT * FROM documents WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Document not found: {}", id))
}

/// The text a document is written from, and the title to give it.
async fn source_text(pool: &SqlitePool, meeting_id: Option<&str>, project_id: Option<&str>) -> Result<(String, String), String> {
    if let Some(mid) = meeting_id {
        let (title, created): (String, String) = sqlx::query_as("SELECT title, created_at FROM meetings WHERE id = ?")
            .bind(mid)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("Meeting not found")?;
        let lines = load_timed_transcript(pool, mid).await.map_err(|e| e.to_string())?;
        if lines.is_empty() {
            return Err("This meeting has no transcript yet".into());
        }
        let mut text = format!("Meeting: {}\nDate: {}\n\n{}", title, &created[..created.len().min(10)], render_transcript(&lines));
        if let Some(notes) = sqlx::query_scalar::<_, Option<String>>("SELECT notes_markdown FROM meeting_notes WHERE meeting_id = ?")
            .bind(mid)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
            .flatten()
            .filter(|n| !n.trim().is_empty())
        {
            text.push_str(&format!("\n\nThe user's own notes from this meeting:\n{}", notes.trim()));
        }
        return Ok((text, title));
    }

    let pid = project_id.ok_or("A document needs a meeting or a project")?;
    let project = crate::projects::get_project(pool, pid).await?;
    let meetings: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT m.id, m.title, m.created_at FROM meeting_projects mp JOIN meetings m ON m.id = mp.meeting_id
         WHERE mp.project_id = ? ORDER BY m.created_at",
    )
    .bind(pid)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    if meetings.is_empty() {
        return Err("This project has no meetings yet".into());
    }
    let today = chrono::Local::now().date_naive();
    let mut text = format!("Project: {}\nToday: {}\n", project.name, today);
    if let Some(d) = &project.description {
        text.push_str(&format!("About: {}\n", d));
    }
    for (mid, title, created) in &meetings {
        text.push_str(&format!("\n## Meeting: {} ({})\n", title, &created[..created.len().min(10)]));
        let summary = sqlx::query_scalar::<_, Option<String>>("SELECT result FROM summary_processes WHERE meeting_id = ?")
            .bind(mid)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
            .flatten()
            .and_then(|r| summary_markdown(&r));
        text.push_str(&summary.unwrap_or_else(|| "(not summarised)".into()));
        let items: Vec<(String, String, Option<String>, Option<String>, Option<String>, String)> = sqlx::query_as(
            "SELECT kind, title, owner, due_at, due_date, status FROM meeting_items
             WHERE meeting_id = ? AND status != 'dismissed' ORDER BY kind, due_at",
        )
        .bind(mid)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        if !items.is_empty() {
            text.push_str("\nRecorded items:\n");
            for (kind, title, owner, due_at, due_date, status) in items {
                let overdue = status == "open" && due_at.as_deref().is_some_and(|d| d < today.format("%Y-%m-%d").to_string().as_str());
                text.push_str(&format!(
                    "- [{}{}] {} (owner: {}, due: {}){}\n",
                    kind,
                    if kind == "action" { format!(", {}", status) } else { String::new() },
                    title,
                    owner.unwrap_or_else(|| "not stated".into()),
                    due_at.or(due_date).unwrap_or_else(|| "not stated".into()),
                    if overdue { " OVERDUE" } else { "" }
                ));
            }
        }
    }
    Ok((text, project.name))
}

/// True when the source contains anything marked local-only.
async fn is_local_only(pool: &SqlitePool, meeting_id: Option<&str>, project_id: Option<&str>) -> bool {
    if let Some(mid) = meeting_id {
        return crate::projects::is_local_only(pool, mid).await;
    }
    let Some(pid) = project_id else { return true };
    sqlx::query_scalar::<_, i64>(
        "SELECT (SELECT COUNT(*) FROM projects WHERE id = ?1 AND privacy = 'local')
              + (SELECT COUNT(*) FROM meeting_projects mp JOIN meeting_projects other ON other.meeting_id = mp.meeting_id
                 JOIN projects p ON p.id = other.project_id WHERE mp.project_id = ?1 AND p.privacy = 'local')",
    )
    .bind(pid)
    .fetch_one(pool)
    .await
    .map(|n| n > 0)
    .unwrap_or(true)
}

async fn generate<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool, doc: &Document) -> Result<(String, String), String> {
    let config = SettingsRepository::get_model_config(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Choose a summary model in Settings first")?;
    let provider = LLMProvider::from_str(&config.provider)?;
    if !matches!(provider, LLMProvider::Ollama | LLMProvider::BuiltInAI)
        && is_local_only(pool, doc.meeting_id.as_deref(), doc.project_id.as_deref()).await
    {
        return Err("This includes a local-only project, so it can't be sent to a cloud model. \
                    Use Ollama or the built-in model, or change the project's privacy setting."
            .into());
    }
    let llm = crate::summary::llm_config::resolve_llm(app, pool, &provider, &config.provider, &config.model).await?;
    let template = crate::summary::templates::get_template(&doc.template_id)?;
    let (text, _) = source_text(pool, doc.meeting_id.as_deref(), doc.project_id.as_deref()).await?;
    let mut prompt = String::new();
    if let Some(mid) = doc.meeting_id.as_deref() {
        if let Some(marks) = crate::recordings::bookmarks::prompt_lines(pool, mid).await {
            prompt = format!("The user bookmarked these moments as important; cover them:\n{}", marks);
        }
    }
    let generated = generate_meeting_summary(
        &reqwest::Client::new(),
        &provider,
        &config.model,
        &llm.api_key,
        &text,
        &prompt,
        &doc.template_id,
        &template,
        llm.token_threshold,
        llm.ollama_endpoint.as_deref(),
        llm.custom_openai_endpoint.as_deref(),
        llm.max_tokens,
        llm.temperature,
        llm.top_p,
        llm.app_data_dir.as_ref(),
        None,
        None,
        None,
        None,
    )
    .await?;
    let cleaned = clean_llm_markdown_detailed(&generated.final_markdown).markdown;
    let body = strip_title(&cleaned);
    if body.trim().is_empty() {
        return Err("The model returned an empty document".into());
    }
    Ok((body, format!("{}:{}", config.provider, config.model)))
}

/// Starts generating a document for a meeting or a project; returns it at once with
/// status 'generating'. A 'document-updated' event fires when it is ready or failed.
#[tauri::command]
pub async fn api_generate_document<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: Option<String>,
    project_id: Option<String>,
    template_id: String,
) -> Result<Document, String> {
    if meeting_id.is_some() == project_id.is_some() {
        return Err("Give either a meeting or a project".into());
    }
    let template = crate::summary::templates::get_template(&template_id)?;
    let pool = state.db_manager.pool().clone();
    let (_, source_title) = source_text(&pool, meeting_id.as_deref(), project_id.as_deref()).await?;
    let ts = now();
    let doc = Document {
        id: Uuid::new_v4().to_string(),
        meeting_id,
        project_id,
        title: format!("{}: {}", template.name, source_title),
        template_id,
        markdown: String::new(),
        status: "generating".into(),
        error: None,
        generated_by: "pending".into(),
        user_edited: false,
        created_at: ts.clone(),
        updated_at: ts,
    };
    sqlx::query(
        "INSERT INTO documents (id, meeting_id, project_id, template_id, title, markdown, status, generated_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, '', 'generating', 'pending', ?, ?)",
    )
    .bind(&doc.id)
    .bind(&doc.meeting_id)
    .bind(&doc.project_id)
    .bind(&doc.template_id)
    .bind(&doc.title)
    .bind(&doc.created_at)
    .bind(&doc.updated_at)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let task_doc = doc.clone();
    tauri::async_runtime::spawn(async move {
        let result = generate(&app, &pool, &task_doc).await;
        let status = match &result {
            Ok(_) => "ready",
            Err(_) => "failed",
        };
        let update = match result {
            Ok((markdown, by)) => sqlx::query(
                "UPDATE documents SET markdown = ?, status = 'ready', error = NULL, generated_by = ?, updated_at = ? WHERE id = ?",
            )
            .bind(markdown)
            .bind(by)
            .bind(now())
            .bind(&task_doc.id)
            .execute(&pool)
            .await,
            Err(e) => {
                error!("Document {} failed: {}", task_doc.id, e);
                sqlx::query("UPDATE documents SET status = 'failed', error = ?, updated_at = ? WHERE id = ?")
                    .bind(e)
                    .bind(now())
                    .bind(&task_doc.id)
                    .execute(&pool)
                    .await
            }
        };
        if let Err(e) = update {
            error!("Could not save document {}: {}", task_doc.id, e);
        }
        info!("Document {} {}", task_doc.id, status);
        let _ = app.emit("document-updated", DocumentEvent { id: task_doc.id.clone(), status: status.into() });
    });
    Ok(doc)
}

#[tauri::command]
pub async fn api_list_documents(
    state: tauri::State<'_, AppState>,
    meeting_id: Option<String>,
    project_id: Option<String>,
) -> Result<Vec<Document>, String> {
    sqlx::query_as::<_, Document>(
        "SELECT * FROM documents WHERE (?1 IS NOT NULL AND meeting_id = ?1) OR (?2 IS NOT NULL AND project_id = ?2)
         ORDER BY created_at DESC",
    )
    .bind(meeting_id)
    .bind(project_id)
    .fetch_all(state.db_manager.pool())
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_get_document(state: tauri::State<'_, AppState>, id: String) -> Result<Document, String> {
    get(state.db_manager.pool(), &id).await
}

#[tauri::command]
pub async fn api_update_document(
    state: tauri::State<'_, AppState>,
    id: String,
    title: Option<String>,
    markdown: Option<String>,
) -> Result<Document, String> {
    let pool = state.db_manager.pool();
    let current = get(pool, &id).await?;
    if current.status == "generating" {
        return Err("Wait until the document has finished generating".into());
    }
    sqlx::query("UPDATE documents SET title = ?, markdown = ?, user_edited = 1, status = 'ready', error = NULL, updated_at = ? WHERE id = ?")
        .bind(title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or(current.title))
        .bind(markdown.unwrap_or(current.markdown))
        .bind(now())
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    get(pool, &id).await
}

#[tauri::command]
pub async fn api_delete_document(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    sqlx::query("DELETE FROM documents WHERE id = ?")
        .bind(&id)
        .execute(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// Templates that make sense for a meeting document, or (for_project) a project document.
#[tauri::command]
pub fn api_list_document_templates(for_project: bool) -> Result<Vec<DocumentTemplate>, String> {
    let all = crate::summary::templates::list_templates();
    Ok(all
        .into_iter()
        .filter(|(id, _, _)| PROJECT_TEMPLATES.contains(&id.as_str()) == for_project)
        .map(|(id, name, description)| DocumentTemplate { id, name, description })
        .collect())
}

/// File name for an export, from the document title: "Meeting Minutes: Budget review" ->
/// "Meeting Minutes - Budget review.docx".
fn export_file_name(title: &str, ext: &str) -> String {
    let safe: String = title
        .replace(": ", " - ")
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c })
        .collect();
    let safe = safe.split_whitespace().collect::<Vec<_>>().join(" ").replace(" - - ", " - ");
    format!("{}.{}", safe.trim_matches(|c: char| c == '-' || c.is_whitespace()), ext)
}

/// Asks where to save, then writes Markdown as is or Word (.docx) converted from the HTML
/// the app rendered, using macOS's built-in textutil. Returns the path, or None if the
/// user cancelled.
#[tauri::command]
pub async fn api_export_document<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    id: String,
    format: String,
    html: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let doc = get(state.db_manager.pool(), &id).await?;
    let (ext, label) = match format.as_str() {
        "md" => ("md", "Markdown"),
        "docx" => ("docx", "Word document"),
        other => return Err(format!("Unknown export format: {}", other)),
    };
    let app_clone = app.clone();
    let file_name = export_file_name(&doc.title, ext);
    let picked = tokio::task::spawn_blocking(move || {
        app_clone.dialog().file().set_file_name(&file_name).add_filter(label, &[ext]).blocking_save_file()
    })
    .await
    .map_err(|e| format!("Save dialog failed: {}", e))?;
    let Some(picked) = picked else { return Ok(None) };
    let path = picked.into_path().map_err(|e| e.to_string())?;

    if format == "md" {
        std::fs::write(&path, format!("# {}\n\n{}\n", doc.title, doc.markdown.trim())).map_err(|e| e.to_string())?;
    } else {
        let html = html.ok_or("Word export needs the rendered document")?;
        let tmp = std::env::temp_dir().join(format!("hearken-export-{}.html", doc.id));
        std::fs::write(&tmp, html).map_err(|e| e.to_string())?;
        let out = std::process::Command::new("/usr/bin/textutil")
            .args(["-convert", "docx", "-output"])
            .arg(&path)
            .arg(&tmp)
            .output()
            .map_err(|e| format!("Could not run textutil: {}", e));
        let _ = std::fs::remove_file(&tmp);
        let out = out?;
        if !out.status.success() {
            return Err(format!("Word export failed: {}", String::from_utf8_lossy(&out.stderr)));
        }
    }
    info!("Exported document {} to {}", doc.id, path.display());
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Opens the document in a print window; the page there calls api_print_window once it
/// has rendered, which shows the macOS print dialog (choose "Save as PDF").
#[tauri::command]
pub async fn api_open_print_window<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    let label = format!("print-{}", &id[..id.len().min(8)]);
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.close();
    }
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(format!("print?id={}", id).into()))
        .title("Print or save as PDF")
        .inner_size(820.0, 1000.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn api_print_window<R: Runtime>(window: tauri::WebviewWindow<R>) -> Result<(), String> {
    window.print().map_err(|e| e.to_string())
}

/// For the MCP server and tests: a ready document written by someone else (e.g. Claude).
pub async fn insert_ready(
    pool: &SqlitePool,
    meeting_id: Option<&str>,
    project_id: Option<&str>,
    template_id: &str,
    title: &str,
    markdown: &str,
    generated_by: &str,
) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let ts = now();
    sqlx::query(
        "INSERT INTO documents (id, meeting_id, project_id, template_id, title, markdown, status, generated_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'ready', ?, ?, ?)",
    )
    .bind(&id)
    .bind(meeting_id)
    .bind(project_id)
    .bind(template_id)
    .bind(title)
    .bind(markdown)
    .bind(generated_by)
    .bind(&ts)
    .bind(&ts)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_names_are_safe() {
        assert_eq!(export_file_name("Meeting Minutes: Budget review", "docx"), "Meeting Minutes - Budget review.docx");
        assert_eq!(export_file_name("Q3/Q4 plan?", "md"), "Q3-Q4 plan.md");
    }

    #[test]
    fn strips_a_leading_title_only() {
        assert_eq!(strip_title("# Minutes\n\n## Attendees\n- A"), "## Attendees\n- A");
        assert_eq!(strip_title("## Attendees\n- A"), "## Attendees\n- A");
    }

    async fn db() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1', 'Budget review', '2026-10-14T10:00:00Z', 'x')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time) VALUES ('t1', 'm1', 'Kofi will send the forecast by Friday.', 'x', 754)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name, created_at, updated_at) VALUES ('p1', 'Budget 2027', 'x', 'x')")
            .execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn meeting_source_has_timed_transcript() {
        let pool = db().await;
        let (text, title) = source_text(&pool, Some("m1"), None).await.unwrap();
        assert_eq!(title, "Budget review");
        assert!(text.contains("[12:34] Kofi will send the forecast by Friday."));
    }

    #[tokio::test]
    async fn project_source_lists_meetings_and_items() {
        let pool = db().await;
        assert!(source_text(&pool, None, Some("p1")).await.is_err(), "no meetings yet");
        crate::projects::set_meeting_projects(&pool, "m1", Some("p1"), &[]).await.unwrap();
        sqlx::query("INSERT INTO meeting_items (id, meeting_id, kind, title, owner, due_at, status, source, evidence_verified, created_at, updated_at)
                     VALUES ('i1', 'm1', 'action', 'Send forecast', 'Kofi', '2026-01-02', 'open', 'ai', 1, 'x', 'x')")
            .execute(&pool).await.unwrap();
        let (text, title) = source_text(&pool, None, Some("p1")).await.unwrap();
        assert_eq!(title, "Budget 2027");
        assert!(text.contains("## Meeting: Budget review (2026-10-14)"));
        assert!(text.contains("Send forecast (owner: Kofi, due: 2026-01-02) OVERDUE"));
    }

    #[tokio::test]
    async fn documents_are_searchable_and_local_only_is_detected() {
        let pool = db().await;
        let id = insert_ready(&pool, Some("m1"), None, "meeting_minutes", "Minutes", "The tamarind contract was signed.", "user").await.unwrap();
        let hits: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM search_index WHERE search_index MATCH 'tamarind' AND source = 'document'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(hits, 1);
        sqlx::query("DELETE FROM documents WHERE id = ?").bind(&id).execute(&pool).await.unwrap();
        let hits: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM search_index WHERE source = 'document'").fetch_one(&pool).await.unwrap();
        assert_eq!(hits, 0);

        assert!(!is_local_only(&pool, None, Some("p1")).await);
        sqlx::query("INSERT INTO projects (id, name, privacy, created_at, updated_at) VALUES ('p2', 'Private', 'local', 'x', 'x')")
            .execute(&pool).await.unwrap();
        crate::projects::set_meeting_projects(&pool, "m1", Some("p1"), &["p2".into()]).await.unwrap();
        assert!(is_local_only(&pool, None, Some("p1")).await, "a meeting also in a local project makes the report local");
    }
}
