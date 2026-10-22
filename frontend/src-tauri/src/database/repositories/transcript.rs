use crate::api::{TranscriptSearchResult, TranscriptSegment};
use chrono::Utc;
use sqlx::{Connection, Error as SqlxError, SqlitePool};
use tracing::{error, info};
use uuid::Uuid;

pub struct TranscriptsRepository;

impl TranscriptsRepository {
    /// Saves a new meeting and its associated transcript segments.
    /// This function uses a transaction to ensure that either both the meeting
    /// and all its transcripts are saved, or none of them are.
    pub async fn save_transcript(
        pool: &SqlitePool,
        meeting_title: &str,
        transcripts: &[TranscriptSegment],
        folder_path: Option<String>,
    ) -> Result<String, SqlxError> {
        let meeting_id = format!("meeting-{}", Uuid::new_v4());

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let now = Utc::now();
        // Dated by when the recording started, not when it was saved.
        let started = folder_path
            .as_deref()
            .and_then(|f| crate::recordings::start_times::recording_started_at(std::path::Path::new(f)))
            .filter(|s| *s <= now)
            .unwrap_or(now);

        // 1. Create the new meeting
        let result = sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&meeting_id)
        .bind(meeting_title)
        .bind(started)
        .bind(now)
        .bind(&folder_path)
        .execute(&mut *transaction)
        .await;

        if let Err(e) = result {
            error!("Failed to create meeting '{}': {}", meeting_title, e);
            transaction.rollback().await?;
            return Err(e);
        }

        info!("Successfully created meeting with id: {}", meeting_id);

        // 2. Save each transcript segment with audio timing fields
        for segment in transcripts {
            let transcript_id = format!("transcript-{}", Uuid::new_v4());
            let result = sqlx::query(
                "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration)
                 VALUES (?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&transcript_id)
            .bind(&meeting_id)
            .bind(&segment.text)
            .bind(&segment.timestamp)
            .bind(segment.audio_start_time)
            .bind(segment.audio_end_time)
            .bind(segment.duration)
            .execute(&mut *transaction)
            .await;

            if let Err(e) = result {
                error!(
                    "Failed to save transcript segment for meeting {}: {}",
                    meeting_id, e
                );
                transaction.rollback().await?;
                return Err(e);
            }
        }

        info!(
            "Successfully saved {} transcript segments for meeting {}",
            transcripts.len(),
            meeting_id
        );

        // Commit the transaction
        transaction.commit().await?;

        Ok(meeting_id)
    }

    /// Searches every meeting's title, transcript, AI summary and notes.
    ///
    /// Uses the FTS5 `search_index` (ranked with BM25, with highlighted snippets).
    /// Falls back to a plain LIKE scan over transcripts if the index is unavailable.
    pub async fn search_transcripts(
        pool: &SqlitePool,
        query: &str,
    ) -> Result<Vec<TranscriptSearchResult>, SqlxError> {
        let Some(fts_query) = Self::build_fts_query(query) else {
            return Ok(Vec::new());
        };

        let fts = sqlx::query_as::<_, (String, String, String, String, String, Option<f64>)>(
            "SELECT s.meeting_id,
                    m.title,
                    snippet(search_index, 0, '**', '**', '…', 24) AS snippet,
                    COALESCE(s.ts, m.created_at) AS ts,
                    s.source,
                    t.audio_start_time
             FROM search_index s
             JOIN meetings m ON m.id = s.meeting_id
             LEFT JOIN transcripts t ON s.source = 'transcript' AND t.id = s.ref_id
             WHERE search_index MATCH ?
             ORDER BY bm25(search_index)
             LIMIT 100",
        )
        .bind(&fts_query)
        .fetch_all(pool)
        .await;

        match fts {
            Ok(rows) => Ok(rows
                .into_iter()
                .map(|(id, title, snippet, timestamp, source, audio_start_time)| TranscriptSearchResult {
                    id,
                    title,
                    match_context: Self::clean_snippet(&snippet, &source),
                    timestamp,
                    source,
                    audio_start_time,
                })
                .collect()),
            Err(e) => {
                error!("Full-text search failed ({}); falling back to LIKE search", e);
                Self::search_transcripts_like(pool, query).await
            }
        }
    }

    /// Turns free text into a safe FTS5 query: every word must appear,
    /// the last word also matches as a prefix ("deploy" finds "deployment").
    pub fn build_fts_query(query: &str) -> Option<String> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|t| t.replace('"', ""))
            .filter(|t| t.chars().any(|c| c.is_alphanumeric()))
            .collect();
        let (last, rest) = terms.split_last()?;
        let mut parts: Vec<String> = rest.iter().map(|t| format!("\"{}\"", t)).collect();
        parts.push(format!("\"{}\"*", last));
        Some(parts.join(" AND "))
    }

    /// Summaries are stored as JSON; strip the JSON punctuation from their snippets.
    fn clean_snippet(snippet: &str, source: &str) -> String {
        if source != "summary" {
            return snippet.to_string();
        }
        let cleaned: String = snippet
            .replace("\\n", " ")
            .chars()
            .filter(|c| !matches!(c, '{' | '}' | '[' | ']' | '"' | '\\'))
            .collect();
        cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Original substring search over transcripts (used only as a fallback).
    async fn search_transcripts_like(
        pool: &SqlitePool,
        query: &str,
    ) -> Result<Vec<TranscriptSearchResult>, SqlxError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let search_query = format!("%{}%", query.to_lowercase());

        let rows = sqlx::query_as::<_, (String, String, String, String, Option<f64>)>(
            "SELECT m.id, m.title, t.transcript, t.timestamp, t.audio_start_time
             FROM meetings m
             JOIN transcripts t ON m.id = t.meeting_id
             WHERE LOWER(t.transcript) LIKE ?",
        )
        .bind(&search_query)
        .fetch_all(pool)
        .await?;

        let results = rows
            .into_iter()
            .map(|(id, title, transcript, timestamp, audio_start_time)| {
                let match_context = Self::get_match_context(&transcript, query);
                TranscriptSearchResult {
                    id,
                    title,
                    match_context,
                    timestamp,
                    source: "transcript".to_string(),
                    audio_start_time,
                }
            })
            .collect();

        Ok(results)
    }

    /// Helper function to extract a snippet of text around the first match of a query.
    fn get_match_context(transcript: &str, query: &str) -> String {
        let transcript_lower = transcript.to_lowercase();
        let query_lower = query.to_lowercase();

        match transcript_lower.find(&query_lower) {
            Some(match_index) => {
                let mut start_index = match_index.saturating_sub(100).min(transcript.len());
                while !transcript.is_char_boundary(start_index) { start_index -= 1; }
                let mut end_index = (match_index + query.len() + 100).min(transcript.len());
                while !transcript.is_char_boundary(end_index) { end_index += 1; }

                let mut context = String::new();
                if start_index > 0 {
                    context.push_str("...");
                }
                context.push_str(&transcript[start_index..end_index]);
                if end_index < transcript.len() {
                    context.push_str("...");
                }
                context
            }
            None => transcript.chars().take(200).collect(), // Fallback to the start of the transcript
        }
    }
}
