//! Local intelligence: search by meaning (embeddings), "Ask Hearken" (answers with
//! citations from your own meetings), a running brief per project, and a weekly digest.
//! The embedding model always runs locally in Ollama; answers and briefs use the summary
//! model you chose (local-only projects never go to a cloud model).

pub mod ask;
pub mod brief;
pub mod digest;
pub mod embeddings;

use crate::database::repositories::setting::SettingsRepository;
use crate::state::AppState;
use crate::summary::llm_client::{generate_summary, LLMProvider};
use crate::summary::llm_config::{resolve_llm, ResolvedLlm};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime};

const SETTINGS_FILE: &str = "intelligence.json";
pub const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";
const DEFAULT_OLLAMA: &str = "http://localhost:11434";
const INDEX_EVERY: std::time::Duration = std::time::Duration::from_secs(20 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntelligenceSettings {
    pub semantic_search: bool,
    pub embed_model: String,
    #[serde(skip)]
    pub ollama_endpoint: String,
}

impl Default for IntelligenceSettings {
    fn default() -> Self {
        Self { semantic_search: false, embed_model: DEFAULT_EMBED_MODEL.into(), ollama_endpoint: DEFAULT_OLLAMA.into() }
    }
}

fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(SETTINGS_FILE))
}

pub async fn load_settings<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool) -> IntelligenceSettings {
    let mut s: IntelligenceSettings = settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    // The same Ollama the summaries use (a custom endpoint if one is set).
    s.ollama_endpoint = SettingsRepository::get_model_config(pool)
        .await
        .ok()
        .flatten()
        .and_then(|c| c.ollama_endpoint)
        .filter(|e| !e.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_OLLAMA.into());
    s
}

fn save_settings<R: Runtime>(app: &AppHandle<R>, s: &IntelligenceSettings) -> Result<(), String> {
    let path = settings_path(app).ok_or("No app data folder")?;
    std::fs::write(path, serde_json::to_string_pretty(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// The user's summary model, ready to call.
pub struct CurrentModel {
    pub provider: LLMProvider,
    pub model: String,
    pub llm: ResolvedLlm,
}

impl CurrentModel {
    pub fn is_local(&self) -> bool {
        matches!(self.provider, LLMProvider::Ollama | LLMProvider::BuiltInAI)
    }

    pub fn label(&self) -> String {
        self.model.clone()
    }

    pub async fn complete(&self, system: &str, user: &str) -> Result<String, String> {
        let c = generate_summary(
            &reqwest::Client::new(),
            &self.provider,
            &self.model,
            &self.llm.api_key,
            system,
            user,
            self.llm.ollama_endpoint.as_deref(),
            self.llm.custom_openai_endpoint.as_deref(),
            self.llm.max_tokens,
            Some(0.2),
            self.llm.top_p,
            self.llm.app_data_dir.as_ref(),
            None,
        )
        .await?;
        Ok(c.content)
    }
}

pub async fn current_model<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool) -> Result<CurrentModel, String> {
    let config = SettingsRepository::get_model_config(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Choose a summary model in Settings first")?;
    let provider = LLMProvider::from_str(&config.provider)?;
    let llm = resolve_llm(app, pool, &provider, &config.provider, &config.model).await?;
    Ok(CurrentModel { provider, model: config.model, llm })
}

/// Meetings linked to any local-only project.
pub async fn local_only_meetings(pool: &SqlitePool) -> std::collections::HashSet<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT mp.meeting_id FROM meeting_projects mp JOIN projects p ON p.id = mp.project_id WHERE p.privacy = 'local'",
    )
    .fetch_all(pool)
    .await
    .map(|v| v.into_iter().collect())
    .unwrap_or_default()
}

async fn model_installed(endpoint: &str, model: &str) -> bool {
    let Ok(resp) = reqwest::Client::new()
        .get(format!("{}/api/tags", endpoint.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    else {
        return false;
    };
    let Ok(v) = resp.json::<serde_json::Value>().await else { return false };
    v["models"].as_array().is_some_and(|ms| {
        ms.iter().any(|m| {
            let name = m["name"].as_str().unwrap_or_default();
            name == model || name.split(':').next() == Some(model)
        })
    })
}

/// Indexes changed meetings in the background (when search by meaning is on) and sends
/// the weekly digest notification on Friday afternoons.
pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(120)).await;
        loop {
            if let Some(state) = app.try_state::<AppState>() {
                let pool = state.db_manager.pool().clone();
                let settings = load_settings(&app, &pool).await;
                if settings.semantic_search {
                    if let Err(e) = embeddings::index_all(&pool, &settings).await {
                        embeddings::warn_once(&e);
                    }
                }
                digest::maybe_notify(&app, &pool).await;
            }
            tokio::time::sleep(INDEX_EVERY).await;
        }
    });
}

/// After a summary: refresh the meeting's index (if on) and its project's brief.
pub async fn after_summary<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool, meeting_id: &str) {
    let settings = load_settings(app, pool).await;
    if settings.semantic_search {
        if let Err(e) = embeddings::index_meeting(pool, &settings, meeting_id).await {
            embeddings::warn_once(&e);
        }
    }
    if let Err(e) = brief::refresh_for_meeting(app, pool, meeting_id).await {
        log::warn!("Project brief not updated after {}: {}", meeting_id, e);
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntelligenceStatus {
    pub semantic_search: bool,
    pub embed_model: String,
    pub model_installed: bool,
    pub indexed_meetings: i64,
    pub total_meetings: i64,
    pub passages: i64,
}

#[tauri::command]
pub async fn api_get_intelligence_status<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>) -> Result<IntelligenceStatus, String> {
    let pool = state.db_manager.pool();
    let s = load_settings(&app, pool).await;
    let (indexed, passages): (i64, i64) = sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(chunks), 0) FROM embedding_status WHERE model = ?")
        .bind(&s.embed_model)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meetings").fetch_one(pool).await.map_err(|e| e.to_string())?;
    Ok(IntelligenceStatus {
        model_installed: model_installed(&s.ollama_endpoint, &s.embed_model).await,
        semantic_search: s.semantic_search,
        embed_model: s.embed_model,
        indexed_meetings: indexed,
        total_meetings: total,
        passages,
    })
}

#[tauri::command]
pub async fn api_set_semantic_search<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let pool = state.db_manager.pool().clone();
    let mut s = load_settings(&app, &pool).await;
    if enabled && !model_installed(&s.ollama_endpoint, &s.embed_model).await {
        return Err(format!("Download the {} model first (it runs locally in Ollama)", s.embed_model));
    }
    s.semantic_search = enabled;
    save_settings(&app, &s)?;
    if enabled {
        tauri::async_runtime::spawn(async move {
            if let Err(e) = embeddings::index_all(&pool, &s).await {
                embeddings::warn_once(&e);
            }
        });
    }
    Ok(())
}

/// Indexes changed meetings now; returns (meetings, passages) newly indexed.
#[tauri::command]
pub async fn api_reindex_now<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>) -> Result<(usize, usize), String> {
    let pool = state.db_manager.pool();
    let s = load_settings(&app, pool).await;
    if !s.semantic_search {
        return Err("Search by meaning is off".into());
    }
    embeddings::index_all(pool, &s).await
}

#[tauri::command]
pub async fn api_semantic_search<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<embeddings::Hit>, String> {
    let pool = state.db_manager.pool();
    let s = load_settings(&app, pool).await;
    if !s.semantic_search {
        return Ok(Vec::new());
    }
    embeddings::search(pool, &s, query.trim(), limit.unwrap_or(10).clamp(1, 50), &Default::default()).await
}
