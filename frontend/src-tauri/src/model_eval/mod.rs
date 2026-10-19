//! Compare transcription models on your own recordings (e.g. Ghanaian-accented English):
//! correct a short stretch of a transcript, and each chosen Whisper / Parakeet model
//! transcribes that same audio. Accuracy is the word error rate (WER) against your text;
//! speed is how much faster than real time it ran. Results add up across clips into a
//! recommendation. Runs only when not recording, and puts your configured models back after.

use crate::audio::retranscription::{find_audio_file, get_or_init_parakeet, get_or_init_whisper};
use crate::state::AppState;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use std::path::PathBuf;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Runtime};
use uuid::Uuid;

const MAX_CLIP_SECONDS: f64 = 600.0;

/// Lowercase words with punctuation removed ("Don't!" -> "don't").
pub fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| c.is_whitespace())
        .map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect::<String>())
        .map(|w| w.trim_matches('\'').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Word-level edit distance: substitutions + deletions + insertions.
pub fn word_errors(reference: &[String], hypothesis: &[String]) -> usize {
    let mut prev: Vec<usize> = (0..=hypothesis.len()).collect();
    let mut cur = vec![0; hypothesis.len() + 1];
    for (i, r) in reference.iter().enumerate() {
        cur[0] = i + 1;
        for (j, h) in hypothesis.iter().enumerate() {
            let cost = if r == h { 0 } else { 1 };
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[hypothesis.len()]
}

pub fn wer(reference: &str, hypothesis: &str) -> (usize, usize, f64) {
    let (r, h) = (words(reference), words(hypothesis));
    let errors = word_errors(&r, &h);
    let rate = if r.is_empty() { if h.is_empty() { 0.0 } else { 1.0 } } else { errors as f64 / r.len() as f64 };
    (errors, r.len(), rate)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChoice {
    /// "whisper" or "parakeet"
    pub engine: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct EvalResult {
    pub id: String,
    pub run_id: String,
    pub meeting_id: Option<String>,
    pub from_seconds: f64,
    pub to_seconds: f64,
    pub engine: String,
    pub model: String,
    pub reference_words: i64,
    pub errors: i64,
    pub wer: f64,
    pub hypothesis: String,
    pub audio_seconds: f64,
    pub seconds_taken: f64,
    pub created_at: String,
}

async fn transcribe<R: Runtime>(app: &AppHandle<R>, choice: &ModelChoice, samples: Vec<f32>) -> Result<String, String> {
    match choice.engine.as_str() {
        "whisper" => {
            let engine = get_or_init_whisper(app, Some(&choice.model)).await.map_err(|e| e.to_string())?;
            engine.transcribe_audio(samples, Some("en".into())).await.map_err(|e| e.to_string())
        }
        "parakeet" => {
            let engine = get_or_init_parakeet(app, Some(&choice.model)).await.map_err(|e| e.to_string())?;
            engine.transcribe_audio(samples).await.map_err(|e| e.to_string())
        }
        other => Err(format!("Unknown engine: {}", other)),
    }
}

async fn meeting_audio(pool: &SqlitePool, meeting_id: &str) -> Result<PathBuf, String> {
    let folder: Option<String> = sqlx::query_scalar::<_, Option<String>>("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .flatten();
    let folder = folder.map(PathBuf::from).filter(|f| f.is_dir()).ok_or("This meeting has no recording folder")?;
    find_audio_file(&folder).map_err(|e| e.to_string())
}

/// Transcribes [from, to] of the meeting with each model and scores it against `reference`.
#[tauri::command]
pub async fn api_compare_models<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    from_seconds: f64,
    to_seconds: f64,
    reference: String,
    models: Vec<ModelChoice>,
) -> Result<Vec<EvalResult>, String> {
    if crate::audio::recording_commands::current_recording_seconds().is_some() {
        return Err("Stop recording first: comparing models swaps the transcription model".into());
    }
    if words(&reference).len() < 20 {
        return Err("Type at least 20 words of the correct transcript for a meaningful score".into());
    }
    if !(to_seconds > from_seconds) || to_seconds - from_seconds > MAX_CLIP_SECONDS {
        return Err(format!("Choose a clip between 1 second and {} minutes long", MAX_CLIP_SECONDS / 60.0));
    }
    if models.is_empty() {
        return Err("Choose at least one model".into());
    }
    let pool = state.db_manager.pool();
    let path = meeting_audio(pool, &meeting_id).await?;
    let samples: Vec<f32> = tokio::task::spawn_blocking(move || -> Result<Vec<f32>, String> {
        let decoded = crate::audio::decoder::decode_audio_file(&path).map_err(|e| e.to_string())?;
        Ok(decoded.to_whisper_format())
    })
    .await
    .map_err(|e| e.to_string())??;
    let (a, b) = (((from_seconds * 16000.0) as usize).min(samples.len()), ((to_seconds * 16000.0) as usize).min(samples.len()));
    if b <= a {
        return Err("That time range is past the end of the recording".into());
    }
    let clip = samples[a..b].to_vec();
    let audio_seconds = clip.len() as f64 / 16000.0;

    let run_id = Uuid::new_v4().to_string();
    let mut results = Vec::new();
    for (i, choice) in models.iter().enumerate() {
        let _ = app.emit("model-compare-progress", serde_json::json!({"index": i, "total": models.len(), "model": choice.model}));
        let started = Instant::now();
        let outcome = transcribe(&app, choice, clip.clone()).await;
        let taken = started.elapsed().as_secs_f64();
        let hypothesis = match outcome {
            Ok(t) => t,
            Err(e) => {
                log::warn!("Model comparison: {} {} failed: {}", choice.engine, choice.model, e);
                format!("(failed: {})", e)
            }
        };
        let (errors, ref_words, rate) = wer(&reference, if hypothesis.starts_with("(failed") { "" } else { &hypothesis });
        let r = EvalResult {
            id: Uuid::new_v4().to_string(),
            run_id: run_id.clone(),
            meeting_id: Some(meeting_id.clone()),
            from_seconds,
            to_seconds,
            engine: choice.engine.clone(),
            model: choice.model.clone(),
            reference_words: ref_words as i64,
            errors: errors as i64,
            wer: rate,
            hypothesis,
            audio_seconds,
            seconds_taken: taken,
            created_at: Utc::now().to_rfc3339(),
        };
        sqlx::query(
            "INSERT INTO model_evals (id, run_id, meeting_id, from_seconds, to_seconds, engine, model, reference_words, errors, wer, hypothesis, audio_seconds, seconds_taken, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&r.id).bind(&r.run_id).bind(&r.meeting_id).bind(r.from_seconds).bind(r.to_seconds).bind(&r.engine).bind(&r.model)
        .bind(r.reference_words).bind(r.errors).bind(r.wer).bind(&r.hypothesis).bind(r.audio_seconds).bind(r.seconds_taken).bind(&r.created_at)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        results.push(r);
    }
    // Put the configured models back for live transcription.
    let _ = get_or_init_whisper(&app, None).await;
    let _ = get_or_init_parakeet(&app, None).await;
    Ok(results)
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ModelScore {
    pub engine: String,
    pub model: String,
    pub clips: i64,
    pub reference_words: i64,
    /// Errors / reference words across every clip
    pub wer: f64,
    /// Audio seconds per second of processing (higher is faster)
    pub speed: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub scores: Vec<ModelScore>,
    /// The most accurate model that runs at least twice as fast as real time
    pub recommended: Option<ModelScore>,
    pub recent: Vec<EvalResult>,
}

pub async fn comparison(pool: &SqlitePool) -> Result<Comparison, String> {
    let scores = sqlx::query_as::<_, ModelScore>(
        "SELECT engine, model, COUNT(DISTINCT run_id) AS clips, SUM(reference_words) AS reference_words,
            CAST(SUM(errors) AS REAL) / MAX(SUM(reference_words), 1) AS wer,
            SUM(audio_seconds) / MAX(SUM(seconds_taken), 0.001) AS speed
         FROM model_evals WHERE hypothesis NOT LIKE '(failed%' GROUP BY engine, model ORDER BY wer",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let recommended = scores
        .iter()
        .filter(|s| s.speed >= 2.0 && s.reference_words >= 100)
        .min_by(|a, b| a.wer.partial_cmp(&b.wer).unwrap_or(std::cmp::Ordering::Equal))
        .map(|s| ModelScore { engine: s.engine.clone(), model: s.model.clone(), ..*s });
    let recent = sqlx::query_as::<_, EvalResult>("SELECT * FROM model_evals ORDER BY created_at DESC LIMIT 30")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Comparison { scores, recommended, recent })
}

#[tauri::command]
pub async fn api_model_comparison(state: tauri::State<'_, AppState>) -> Result<Comparison, String> {
    comparison(state.db_manager.pool()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_error_rate_counts_edits_and_ignores_case_and_punctuation() {
        assert_eq!(wer("Kofi will send the forecast.", "kofi will send the forecast"), (0, 5, 0.0));
        let (e, n, r) = wer("Kofi will send the forecast by Friday", "Coffee will send forecast by Friday please");
        assert_eq!((e, n), (3, 7), "one substitution, one deletion, one insertion");
        assert!((r - 3.0 / 7.0).abs() < 1e-9);
        assert_eq!(words("Don't stop!  'quoted'"), vec!["don't", "stop", "quoted"]);
    }

    #[tokio::test]
    async fn comparison_weights_by_words_and_recommends_fast_accurate_models() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for (engine, model, words, errors, audio, taken) in [
            ("whisper", "large-v3", 200, 10, 60.0, 40.0),   // 5% WER, 1.5x: too slow for live use
            ("whisper", "small", 200, 30, 60.0, 6.0),       // 15% WER, 10x
            ("parakeet", "tdt-0.6b", 200, 16, 60.0, 3.0),   // 8% WER, 20x
        ] {
            sqlx::query("INSERT INTO model_evals (id, run_id, from_seconds, to_seconds, engine, model, reference_words, errors, wer, hypothesis, audio_seconds, seconds_taken, created_at)
                         VALUES (?, 'r1', 0, 60, ?, ?, ?, ?, ?, 'text', ?, ?, 'x')")
                .bind(format!("{model}")).bind(engine).bind(model).bind(words).bind(errors).bind(errors as f64 / words as f64).bind(audio).bind(taken)
                .execute(&pool).await.unwrap();
        }
        let c = comparison(&pool).await.unwrap();
        assert_eq!(c.scores[0].model, "large-v3", "most accurate first");
        assert_eq!(c.recommended.unwrap().model, "tdt-0.6b", "most accurate of the fast ones");
    }
}
