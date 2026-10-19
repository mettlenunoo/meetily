//! Search by meaning. Each meeting is cut into passages (about 90 seconds of transcript,
//! summary sections, notes, items, documents), each passage gets a vector from a small
//! local embedding model in Ollama, and a question finds the passages whose vectors are
//! closest. Everything stays on the Mac. Vectors are compared in Rust (cosine similarity
//! on normalised vectors), which is fast enough at personal scale and needs no extension.

use crate::intelligence::IntelligenceSettings;
use chrono::Utc;
use hearken_core::export::summary_markdown;
use hearken_core::items::format_clock;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::time::Duration;

/// A transcript passage ends after this many words or seconds, whichever comes first.
const PASSAGE_WORDS: usize = 120;
const PASSAGE_SECONDS: f64 = 90.0;
/// Longer text (summaries, documents) is split into pieces of about this many words.
const PIECE_WORDS: usize = 160;
const EMBED_BATCH: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct Passage {
    pub source: String,
    pub ref_id: Option<String>,
    pub start_seconds: Option<f64>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub meeting_id: String,
    pub meeting_title: String,
    pub meeting_date: String,
    pub source: String,
    pub start_seconds: Option<f64>,
    pub text: String,
    pub score: f32,
}

/// Models with recommended task prefixes (nomic-embed-text was trained with these).
fn prefixes(model: &str) -> (&'static str, &'static str) {
    if model.starts_with("nomic-embed-text") {
        ("search_document: ", "search_query: ")
    } else {
        ("", "")
    }
}

fn split_words(text: &str, max_words: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for para in text.split("\n\n") {
        let words: Vec<&str> = para.split_whitespace().collect();
        if current.len() + words.len() > max_words && !current.is_empty() {
            pieces.push(current.join(" "));
            current.clear();
        }
        for chunk in words.chunks(max_words) {
            if chunk.len() == max_words {
                pieces.push(chunk.join(" "));
            } else {
                current.extend_from_slice(chunk);
            }
        }
    }
    if !current.is_empty() {
        pieces.push(current.join(" "));
    }
    pieces.into_iter().filter(|p| p.split_whitespace().count() >= 3).collect()
}

/// Groups transcript lines into passages of up to PASSAGE_WORDS words / PASSAGE_SECONDS.
pub fn transcript_passages(lines: &[(String, Option<f64>)]) -> Vec<Passage> {
    let mut out = Vec::new();
    let mut words = 0;
    let mut start: Option<f64> = None;
    let mut text = String::new();
    for (line, at) in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let too_long = words >= PASSAGE_WORDS
            || matches!((start, at), (Some(s), Some(a)) if a - s >= PASSAGE_SECONDS);
        if too_long && !text.is_empty() {
            out.push(Passage { source: "transcript".into(), ref_id: None, start_seconds: start, text: std::mem::take(&mut text) });
            words = 0;
            start = None;
        }
        if start.is_none() {
            start = *at;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(line);
        words += line.split_whitespace().count();
    }
    if !text.is_empty() {
        out.push(Passage { source: "transcript".into(), ref_id: None, start_seconds: start, text });
    }
    out
}

pub async fn meeting_passages(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<Passage>, String> {
    let lines: Vec<(String, Option<f64>)> = sqlx::query_as(
        "SELECT transcript, audio_start_time FROM transcripts WHERE meeting_id = ? ORDER BY COALESCE(audio_start_time, 0), timestamp",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let mut out = transcript_passages(&lines);

    if let Some(summary) = sqlx::query_scalar::<_, Option<String>>("SELECT result FROM summary_processes WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .flatten()
        .and_then(|r| summary_markdown(&r))
    {
        out.extend(split_words(&summary, PIECE_WORDS).into_iter().map(|t| Passage { source: "summary".into(), ref_id: None, start_seconds: None, text: t }));
    }
    if let Some(notes) = sqlx::query_scalar::<_, Option<String>>("SELECT notes_markdown FROM meeting_notes WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .flatten()
    {
        out.extend(split_words(&notes, PIECE_WORDS).into_iter().map(|t| Passage { source: "notes".into(), ref_id: None, start_seconds: None, text: t }));
    }
    let items: Vec<(String, String, String, Option<String>, Option<String>, Option<f64>)> = sqlx::query_as(
        "SELECT id, kind, title, owner, evidence_quote, evidence_start_time FROM meeting_items WHERE meeting_id = ? AND status != 'dismissed'",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    for (id, kind, title, owner, quote, at) in items {
        let mut text = format!("{}: {}", if kind == "action" { "Action item" } else { "Decision" }, title);
        if let Some(o) = owner {
            text.push_str(&format!(" (owner: {})", o));
        }
        if let Some(q) = quote {
            text.push_str(&format!(". Said: \"{}\"", q));
        }
        out.push(Passage { source: "item".into(), ref_id: Some(id), start_seconds: at, text });
    }
    let has_documents: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'documents'")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    if has_documents > 0 {
        let docs: Vec<(String, String, String)> = sqlx::query_as("SELECT id, title, markdown FROM documents WHERE meeting_id = ? AND status = 'ready'")
            .bind(meeting_id)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
        for (id, title, md) in docs {
            out.extend(split_words(&md, PIECE_WORDS).into_iter().map(|t| Passage {
                source: "document".into(),
                ref_id: Some(id.clone()),
                start_seconds: None,
                text: format!("{}: {}", title, t),
            }));
        }
    }
    Ok(out)
}

fn hash_passages(model: &str, passages: &[Passage]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    model.hash(&mut h);
    for p in passages {
        p.source.hash(&mut h);
        p.text.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

pub fn normalise(mut v: Vec<f32>) -> Vec<f32> {
    let len = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if len > 0.0 {
        v.iter_mut().for_each(|x| *x /= len);
    }
    v
}

fn to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn dot_bytes(a: &[f32], b: &[u8]) -> f32 {
    a.iter()
        .zip(b.chunks_exact(4))
        .map(|(x, c)| x * f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .sum()
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

/// Vectors for `texts` from Ollama's /api/embed (local).
pub async fn embed(endpoint: &str, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(120)).build().map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(texts.len());
    for batch in texts.chunks(EMBED_BATCH) {
        let resp = client
            .post(format!("{}/api/embed", endpoint.trim_end_matches('/')))
            .json(&serde_json::json!({"model": model, "input": batch}))
            .send()
            .await
            .map_err(|e| format!("Ollama isn't reachable for embeddings: {}", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Embedding failed ({}): {}", status, body.chars().take(200).collect::<String>()));
        }
        let parsed: EmbedResponse = resp.json().await.map_err(|e| e.to_string())?;
        if parsed.embeddings.len() != batch.len() {
            return Err("Ollama returned the wrong number of embeddings".into());
        }
        out.extend(parsed.embeddings.into_iter().map(normalise));
    }
    Ok(out)
}

/// (Re)indexes one meeting if its content changed. Returns the number of passages embedded.
pub async fn index_meeting(pool: &SqlitePool, settings: &IntelligenceSettings, meeting_id: &str) -> Result<usize, String> {
    let passages = meeting_passages(pool, meeting_id).await?;
    let hash = hash_passages(&settings.embed_model, &passages);
    let current: Option<String> = sqlx::query_scalar("SELECT content_hash FROM embedding_status WHERE meeting_id = ? AND model = ?")
        .bind(meeting_id)
        .bind(&settings.embed_model)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    if current.as_deref() == Some(hash.as_str()) {
        return Ok(0);
    }
    let (doc_prefix, _) = prefixes(&settings.embed_model);
    let texts: Vec<String> = passages.iter().map(|p| format!("{}{}", doc_prefix, p.text)).collect();
    let vectors = if texts.is_empty() { Vec::new() } else { embed(&settings.ollama_endpoint, &settings.embed_model, &texts).await? };

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM embedding_chunks WHERE meeting_id = ?").bind(meeting_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    for (p, v) in passages.iter().zip(vectors.iter()) {
        sqlx::query("INSERT INTO embedding_chunks (meeting_id, source, ref_id, start_seconds, text, model, vector) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(meeting_id)
            .bind(&p.source)
            .bind(&p.ref_id)
            .bind(p.start_seconds)
            .bind(&p.text)
            .bind(&settings.embed_model)
            .bind(to_bytes(v))
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    sqlx::query("INSERT OR REPLACE INTO embedding_status (meeting_id, model, content_hash, chunks, indexed_at) VALUES (?, ?, ?, ?, ?)")
        .bind(meeting_id)
        .bind(&settings.embed_model)
        .bind(&hash)
        .bind(passages.len() as i64)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(passages.len())
}

/// Indexes every meeting that changed. Stops at the first Ollama error (it'll retry later).
pub async fn index_all(pool: &SqlitePool, settings: &IntelligenceSettings) -> Result<(usize, usize), String> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM meetings ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let (mut meetings, mut passages) = (0, 0);
    for id in ids {
        let n = index_meeting(pool, settings, &id).await?;
        if n > 0 {
            meetings += 1;
            passages += n;
        }
    }
    if meetings > 0 {
        info!("Indexed {} meeting(s), {} passage(s) for search by meaning", meetings, passages);
    }
    Ok((meetings, passages))
}

/// Passages closest in meaning to `query`. `exclude` drops meetings (e.g. local-only
/// ones when the answer will come from a cloud model).
pub async fn search(
    pool: &SqlitePool,
    settings: &IntelligenceSettings,
    query: &str,
    limit: usize,
    exclude: &std::collections::HashSet<String>,
) -> Result<Vec<Hit>, String> {
    let (_, query_prefix) = prefixes(&settings.embed_model);
    let q = embed(&settings.ollama_endpoint, &settings.embed_model, &[format!("{}{}", query_prefix, query)])
        .await?
        .pop()
        .ok_or("No embedding for the question")?;
    let rows: Vec<(String, String, String, Option<f64>, String, Vec<u8>, String)> = sqlx::query_as(
        "SELECT c.meeting_id, m.title, m.created_at, c.start_seconds, c.text, c.vector, c.source
         FROM embedding_chunks c JOIN meetings m ON m.id = c.meeting_id WHERE c.model = ?",
    )
    .bind(&settings.embed_model)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let mut hits: Vec<Hit> = rows
        .into_iter()
        .filter(|r| !exclude.contains(&r.0) && r.5.len() == q.len() * 4)
        .map(|(meeting_id, title, created, start, text, vector, source)| Hit {
            score: dot_bytes(&q, &vector),
            meeting_date: created.chars().take(10).collect(),
            meeting_id,
            meeting_title: title,
            source,
            start_seconds: start,
            text,
        })
        .collect();
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit);
    Ok(hits)
}

pub fn describe_hit(h: &Hit) -> String {
    let at = h.start_seconds.map(|s| format!(" at [{}]", format_clock(s))).unwrap_or_default();
    format!("\"{}\" ({}){} — {}", h.meeting_title, h.meeting_date, at, h.source)
}

pub fn warn_once(msg: &str) {
    warn!("Search by meaning: {}", msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_passages_break_on_time_and_length() {
        let lines: Vec<(String, Option<f64>)> = (0..10).map(|i| (format!("line {i} has five words"), Some(i as f64 * 20.0))).collect();
        let p = transcript_passages(&lines);
        assert_eq!(p.len(), 2, "180 s of talk splits after 90 s");
        assert_eq!(p[0].start_seconds, Some(0.0));
        assert_eq!(p[1].start_seconds, Some(100.0));
        assert!(p[0].text.starts_with("line 0 has five words line 1"));
    }

    #[test]
    fn long_text_is_split_into_pieces() {
        let text = (0..400).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
        let pieces = split_words(&text, 160);
        assert_eq!(pieces.len(), 3);
        assert!(pieces.iter().all(|p| p.split_whitespace().count() <= 160));
        assert!(split_words("a b", 160).is_empty(), "too short to be useful");
    }

    #[test]
    fn vectors_round_trip_and_score_by_cosine() {
        let a = normalise(vec![3.0, 4.0]);
        assert!((a[0] - 0.6).abs() < 1e-6 && (a[1] - 0.8).abs() < 1e-6);
        assert!((dot_bytes(&a, &to_bytes(&a)) - 1.0).abs() < 1e-6);
        let b = normalise(vec![-4.0, 3.0]);
        assert!(dot_bytes(&a, &to_bytes(&b)).abs() < 1e-6, "orthogonal");
    }

    #[test]
    fn nomic_gets_task_prefixes() {
        assert_eq!(prefixes("nomic-embed-text:latest").1, "search_query: ");
        assert_eq!(prefixes("bge-m3").0, "");
    }
}
