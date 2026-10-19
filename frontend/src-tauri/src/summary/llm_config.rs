//! Resolves everything needed to call the user's configured summary model: API key,
//! Ollama or custom endpoint, sampling settings and how much text fits in one pass.
//! Shared by meeting summaries and document generation.

use crate::database::repositories::setting::SettingsRepository;
use crate::summary::llm_client::LLMProvider;
use crate::ollama::metadata::ModelMetadataCache;
use log::{info, warn};
use sqlx::SqlitePool;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime};

/// Ollama model context sizes, cached for five minutes.
static METADATA_CACHE: std::sync::LazyLock<ModelMetadataCache> =
    std::sync::LazyLock::new(|| ModelMetadataCache::new(std::time::Duration::from_secs(300)));

pub struct ResolvedLlm {
    pub api_key: String,
    pub ollama_endpoint: Option<String>,
    pub custom_openai_endpoint: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    /// Tokens that fit in one pass (longer text is chunked for local models)
    pub token_threshold: usize,
    pub app_data_dir: Option<PathBuf>,
}

pub async fn resolve_llm<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    provider: &LLMProvider,
    model_provider: &str,
    model_name: &str,
) -> Result<ResolvedLlm, String> {
    // Validate and setup api_key, Flexible for Ollama, BuiltInAI, and CustomOpenAI
    let api_key = if *provider == LLMProvider::Ollama || *provider == LLMProvider::BuiltInAI || *provider == LLMProvider::CustomOpenAI {
        // These providers don't require API keys from the standard database column
        String::new()
    } else {
        match SettingsRepository::get_api_key(pool, model_provider).await {
            Ok(Some(key)) if !key.is_empty() => key,
            Ok(None) | Ok(Some(_)) => {
                return Err(format!("API key not found for {}", model_provider));
            }
            Err(e) => {
                return Err(format!("Failed to retrieve API key for {}: {}", model_provider, e));
            }
        }
    };

    // Get Ollama endpoint if provider is Ollama
    let ollama_endpoint = if *provider == LLMProvider::Ollama {
        match SettingsRepository::get_model_config(pool).await {
            Ok(Some(config)) => config.ollama_endpoint,
            Ok(None) => None,
            Err(e) => {
                info!("Failed to retrieve Ollama endpoint: {}, using default", e);
                None
            }
        }
    } else {
        None
    };

    // Get CustomOpenAI config if provider is CustomOpenAI
    let (custom_openai_endpoint, custom_openai_api_key, custom_openai_max_tokens, custom_openai_temperature, custom_openai_top_p) =
        if *provider == LLMProvider::CustomOpenAI {
            match SettingsRepository::get_custom_openai_config(pool).await {
                Ok(Some(config)) => {
                    info!("✓ Using custom OpenAI endpoint: {}", config.endpoint);
                    (
                        Some(config.endpoint),
                        config.api_key,
                        config.max_tokens.map(|t| t as u32),
                        config.temperature,
                        config.top_p,
                    )
                }
                Ok(None) => {
                    return Err("Custom OpenAI provider selected but no configuration found".to_string());
                }
                Err(e) => {
                    return Err(format!("Failed to retrieve custom OpenAI config: {}", e));
                }
            }
        } else {
            (None, None, None, None, None)
        };

    // For CustomOpenAI, use its API key (if any) instead of the empty string
    let final_api_key = if *provider == LLMProvider::CustomOpenAI {
        custom_openai_api_key.unwrap_or_default()
    } else {
        api_key
    };

    // Dynamically fetch context size based on provider and model
    let token_threshold = if *provider == LLMProvider::Ollama {
        match METADATA_CACHE.get_or_fetch(model_name, ollama_endpoint.as_deref()).await {
            Ok(metadata) => {
                // Reserve 300 tokens for prompt overhead
                let optimal = metadata.context_size.saturating_sub(300);
                info!(
                    "✓ Using dynamic context for {}: {} tokens (chunk size: {})",
                    model_name, metadata.context_size, optimal
                );
                optimal
            }
            Err(e) => {
                warn!(
                    "Failed to fetch context for {}: {}. Using default 4000",
                    model_name, e
                );
                4000  // Fallback to safe default
            }
        }
    } else if *provider == LLMProvider::BuiltInAI {
        // Get model's context size from registry
        use crate::summary::summary_engine::models;
        let model = models::get_model_by_name(model_name)
            .ok_or_else(|| format!("Unknown model: {}", model_name));

        match model {
            Ok(model_def) => {
                // Reserve 300 tokens for prompt overhead
                let optimal = model_def.context_size.saturating_sub(300) as usize;
                info!(
                    "✓ Using BuiltInAI context size: {} tokens (chunk size: {})",
                    model_def.context_size, optimal
                );
                optimal
            }
            Err(e) => {
                warn!("{}, using default 2048", e);
                1748  // 2048 - 300 for overhead
            }
        }
    } else {
        // Cloud providers (OpenAI, Claude, Groq, CustomOpenAI) handle large contexts automatically
        100000  // Effectively unlimited for single-pass processing
    };

    // Get app data directory for BuiltInAI provider
    let app_data_dir = app.path().app_data_dir().ok();

    Ok(ResolvedLlm {
        api_key: final_api_key,
        ollama_endpoint,
        custom_openai_endpoint,
        max_tokens: custom_openai_max_tokens,
        temperature: custom_openai_temperature,
        top_p: custom_openai_top_p,
        token_threshold,
        app_data_dir,
    })
}
