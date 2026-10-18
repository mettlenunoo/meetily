//! Suggests a main project for a meeting. Never links anything itself: the suggestion
//! waits for the user to accept or dismiss it.
//!
//! 1. Matching (no model): project names, keywords and people found in the meeting
//!    title and transcript. Used only when one project clearly wins.
//! 2. Model (after a summary, only if matching was unclear): the summary model picks a
//!    project from the list or answers "none". Projects marked local-only are never
//!    sent to a cloud model.

use super::Project;
use crate::summary::llm_client::{generate_summary, LLMProvider};
use crate::summary::meeting_items::LlmTarget;
use chrono::Utc;
use hearken_core::items::normalize;
use log::{info, warn};
use serde::Deserialize;
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

/// A winner needs at least this score and twice the runner-up's.
const MIN_SCORE: i64 = 6;
/// Transcript text sent to the model (the summary is preferred when there is one).
const MAX_PROMPT_CHARS: usize = 12_000;

#[derive(Debug)]
struct Scored {
    project_id: String,
    score: i64,
    reasons: Vec<String>,
}

fn terms(list: &Option<String>) -> Vec<String> {
    list.as_deref()
        .unwrap_or("")
        .split(',')
        .map(normalize)
        .filter(|t| t.chars().count() >= 3)
        .collect()
}

/// Whole-word (or whole-phrase) occurrences of `term` in normalized `text`.
fn count(text: &str, term: &str) -> i64 {
    if term.is_empty() {
        return 0;
    }
    let padded = format!(" {} ", text);
    padded.matches(&format!(" {} ", term)).count() as i64
}

fn score(project: &Project, title: &str, transcript: &str) -> Scored {
    let mut score = 0;
    let mut reasons = Vec::new();

    let name = normalize(&project.name);
    if name.chars().count() >= 3 {
        if count(title, &name) > 0 {
            score += 6;
            reasons.push(format!("“{}” is in the title", project.name));
        }
        let said = count(transcript, &name);
        if said > 0 {
            score += 2 * said.min(5);
            reasons.push(format!("“{}” mentioned {}×", project.name, said));
        }
    }

    let mut keyword_hits = Vec::new();
    for k in terms(&project.keywords) {
        let n = count(title, &k) * 2 + count(transcript, &k);
        if n > 0 {
            score += 3 + n.min(5);
            keyword_hits.push(k);
        }
    }
    if !keyword_hits.is_empty() {
        reasons.push(format!("keywords: {}", keyword_hits.join(", ")));
    }

    let mut people_hits = Vec::new();
    for p in terms(&project.people) {
        // First names are what people say ("Kofi", not "Kofi Mensah")
        let first = p.split(' ').next().unwrap_or(&p).to_string();
        if count(title, &p) + count(transcript, &p) > 0 || (first.chars().count() >= 3 && count(transcript, &first) > 0) {
            score += 2;
            people_hits.push(p);
        }
    }
    if !people_hits.is_empty() {
        reasons.push(format!("people: {}", people_hits.join(", ")));
    }

    Scored { project_id: project.id.clone(), score, reasons }
}

/// The clear winner, if there is one.
fn pick(mut scored: Vec<Scored>) -> Option<Scored> {
    scored.sort_by(|a, b| b.score.cmp(&a.score));
    let mut it = scored.into_iter();
    let best = it.next()?;
    let second = it.next().map(|s| s.score).unwrap_or(0);
    (best.score >= MIN_SCORE && best.score >= 2 * second).then_some(best)
}

async fn active_projects(pool: &SqlitePool) -> Result<Vec<Project>, String> {
    sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE status = 'active'")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())
}

/// True when the meeting already has a main project or Hearken already suggested one
/// (accepted, dismissed or pending): each meeting is asked about once.
async fn already_handled(pool: &SqlitePool, meeting_id: &str) -> Result<bool, String> {
    let n: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM meeting_projects WHERE meeting_id = ?1 AND is_primary = 1)
              + (SELECT COUNT(*) FROM project_suggestions WHERE meeting_id = ?1)",
    )
    .bind(meeting_id)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

async fn meeting_text(pool: &SqlitePool, meeting_id: &str) -> Result<(String, String), String> {
    let title: String = sqlx::query_scalar("SELECT title FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Meeting not found")?;
    let lines: Vec<String> = sqlx::query_scalar(
        "SELECT transcript FROM transcripts WHERE meeting_id = ? ORDER BY COALESCE(audio_start_time, 0), timestamp",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok((title, lines.join(" ")))
}

async fn store(pool: &SqlitePool, meeting_id: &str, project_id: &str, reason: &str, source: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT OR IGNORE INTO project_suggestions (meeting_id, project_id, reason, source, status, created_at)
         VALUES (?, ?, ?, ?, 'pending', ?)",
    )
    .bind(meeting_id)
    .bind(project_id)
    .bind(reason)
    .bind(source)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Matching pass. Returns true if a suggestion was stored.
pub async fn suggest_by_matching(pool: &SqlitePool, meeting_id: &str) -> Result<bool, String> {
    if already_handled(pool, meeting_id).await? {
        return Ok(false);
    }
    let projects = active_projects(pool).await?;
    if projects.is_empty() {
        return Ok(false);
    }
    let (title, transcript) = meeting_text(pool, meeting_id).await?;
    let (title, transcript) = (normalize(&title), normalize(&transcript));
    let scored = projects.iter().map(|p| score(p, &title, &transcript)).collect();
    match pick(scored) {
        Some(best) => {
            store(pool, meeting_id, &best.project_id, &best.reasons.join("; "), "match").await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

#[derive(Deserialize)]
struct ModelChoice {
    project: Option<String>,
    reason: Option<String>,
}

fn build_prompt(projects: &[Project], title: &str, content: &str) -> String {
    let list = projects
        .iter()
        .map(|p| {
            let mut line = format!("- {} ({})", p.name, p.kind);
            if let Some(d) = &p.description { line.push_str(&format!(": {}", d)); }
            if let Some(k) = &p.keywords { line.push_str(&format!(" [keywords: {}]", k)); }
            if let Some(pe) = &p.people { line.push_str(&format!(" [people: {}]", pe)); }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"Which of these projects is this meeting about?

Projects:
{list}

Meeting title: {title}

<meeting>
{content}
</meeting>

Answer with the project's exact name from the list, or null if none clearly fits or you are unsure. Do not guess.
Return only JSON: {{"project": "<exact name or null>", "reason": "<one short sentence citing what in the meeting points to it>"}}"#
    )
}

fn parse_choice(raw: &str) -> Option<ModelChoice> {
    let (start, end) = (raw.find('{')?, raw.rfind('}')?);
    (end > start).then(|| serde_json::from_str(&raw[start..=end]).ok()).flatten()
}

/// Matching first, then the model if matching was unclear. Runs after a summary is saved;
/// failures are logged only.
pub async fn suggest_after_summary(
    pool: &SqlitePool,
    meeting_id: &str,
    llm: &LlmTarget<'_>,
    cancellation_token: Option<&CancellationToken>,
) {
    let run = async {
        if suggest_by_matching(pool, meeting_id).await? || already_handled(pool, meeting_id).await? {
            return Ok::<_, String>(());
        }
        let local_model = matches!(llm.provider, LLMProvider::Ollama | LLMProvider::BuiltInAI);
        // Local-only projects never go to a cloud model.
        let candidates: Vec<Project> = active_projects(pool)
            .await?
            .into_iter()
            .filter(|p| local_model || p.privacy != "local")
            .collect();
        if candidates.is_empty() {
            return Ok(());
        }
        let (title, transcript) = meeting_text(pool, meeting_id).await?;
        let stored: Option<Option<String>> =
            sqlx::query_scalar::<_, Option<String>>("SELECT result FROM summary_processes WHERE meeting_id = ?")
                .bind(meeting_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| e.to_string())?;
        let summary = stored.flatten().and_then(|r| hearken_core::export::summary_markdown(&r));
        let content: String = summary.unwrap_or(transcript).chars().take(MAX_PROMPT_CHARS).collect();

        let completion = generate_summary(
            &reqwest::Client::new(),
            llm.provider,
            llm.model_name,
            llm.api_key,
            "You file meetings under the right project. You output only valid JSON and never guess.",
            &build_prompt(&candidates, &title, &content),
            llm.ollama_endpoint,
            llm.custom_openai_endpoint,
            llm.max_tokens,
            Some(0.1),
            llm.top_p,
            llm.app_data_dir,
            cancellation_token,
        )
        .await?;
        let Some(choice) = parse_choice(&completion.content) else {
            return Err("model returned no usable JSON".into());
        };
        let Some(name) = choice.project.map(|n| n.trim().to_string()).filter(|n| !n.is_empty() && n != "null") else {
            info!("No project suggested for {} (model unsure)", meeting_id);
            return Ok(());
        };
        // Only a project from the list counts; anything else is ignored.
        match candidates.iter().find(|p| p.name.eq_ignore_ascii_case(&name)) {
            Some(p) => {
                let reason = choice.reason.unwrap_or_default();
                store(pool, meeting_id, &p.id, reason.trim(), "ai").await?;
                info!("Suggested project {} for {}", p.name, meeting_id);
            }
            None => warn!("Model suggested unknown project {:?} for {}; ignored", name, meeting_id),
        }
        Ok(())
    };
    if let Err(e) = run.await {
        warn!("Project suggestion failed for {}: {}", meeting_id, e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, name: &str, keywords: Option<&str>, people: Option<&str>) -> Project {
        Project {
            id: id.into(),
            name: name.into(),
            kind: "work".into(),
            description: None,
            people: people.map(Into::into),
            keywords: keywords.map(Into::into),
            slack_channels: None,
            status: "active".into(),
            privacy: "claude".into(),
            color: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn counts_whole_words_only() {
        assert_eq!(count("the gis map and gis data", "gis"), 2);
        assert_eq!(count("logistics", "gis"), 0);
        assert_eq!(count("ifdc geoportal launch", "ifdc geoportal"), 1);
    }

    #[test]
    fn title_and_keywords_make_a_clear_winner() {
        let geo = project("p1", "IFDC Geoportal", Some("GIS, map layers"), Some("Kofi Mensah"));
        let pos = project("p2", "Surveyor POS", Some("receipts"), None);
        let title = normalize("IFDC Geoportal weekly");
        let text = normalize("We reviewed the GIS map layers with Kofi and agreed on the basemap.");
        let best = pick(vec![score(&geo, &title, &text), score(&pos, &title, &text)]).unwrap();
        assert_eq!(best.project_id, "p1");
        assert!(best.reasons.iter().any(|r| r.contains("title")));
        assert!(best.reasons.iter().any(|r| r.contains("people: kofi mensah")));
    }

    #[test]
    fn no_suggestion_when_unclear_or_weak() {
        let a = project("p1", "Alpha", Some("budget"), None);
        let b = project("p2", "Beta", Some("budget"), None);
        let text = normalize("We discussed the budget.");
        assert!(pick(vec![score(&a, "", &text), score(&b, "", &text)]).is_none(), "tie");
        assert!(pick(vec![score(&a, "", &normalize("nothing relevant"))]).is_none(), "too weak");
    }

    #[test]
    fn model_answers_are_parsed_strictly() {
        let c = parse_choice("Sure: {\"project\": \"IFDC Geoportal\", \"reason\": \"GIS work\"}").unwrap();
        assert_eq!(c.project.as_deref(), Some("IFDC Geoportal"));
        assert!(parse_choice("{\"project\": null}").unwrap().project.is_none());
        assert!(parse_choice("no json").is_none());
    }
}
