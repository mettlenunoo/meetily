//! "Ask Hearken": answers a question from the user's own meetings. Passages are found by
//! keyword search (always) and by meaning (when it is on), numbered, and given to the
//! summary model, which must cite them like [2] or answer NOT_FOUND. Citations that don't
//! point at a real excerpt are dropped, and an answer with none is not presented as found.

use super::embeddings::{self, Hit};
use super::{current_model, load_settings, local_only_meetings};
use crate::state::AppState;
use hearken_core::items::format_clock;
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashSet;
use tauri::{AppHandle, Runtime};

const MAX_EXCERPTS: usize = 14;
const MAX_EXCERPT_CHARS: usize = 700;
const NOT_FOUND: &str = "NOT_FOUND";

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "about", "as", "at", "be", "by", "did", "do", "does", "for", "from", "how", "i", "in", "is",
    "it", "me", "my", "of", "on", "or", "our", "so", "that", "the", "this", "to", "us", "was", "we", "were", "what",
    "when", "where", "which", "who", "why", "will", "with", "you", "your", "have", "has", "had", "any", "all", "tell",
    "say", "said", "there", "they", "them", "their", "can", "could", "would", "should",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Excerpt {
    pub n: usize,
    pub meeting_id: String,
    pub meeting_title: String,
    pub meeting_date: String,
    pub source: String,
    pub start_seconds: Option<f64>,
    pub text: String,
    pub cited: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub found: bool,
    /// Markdown with [n] citations into `excerpts`
    pub answer: String,
    pub excerpts: Vec<Excerpt>,
    pub model: String,
    pub used_meaning_search: bool,
}

/// Keyword search terms: any of the question's content words, prefix-matched.
pub fn fts_query(question: &str) -> Option<String> {
    let words: Vec<String> = question
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.chars().count() >= 3 && !STOP_WORDS.contains(&w.as_str()))
        .collect();
    if words.is_empty() {
        return None;
    }
    Some(words.iter().map(|w| format!("\"{}\"*", w)).collect::<Vec<_>>().join(" OR "))
}

async fn keyword_hits(pool: &SqlitePool, question: &str, project_meetings: Option<&HashSet<String>>, exclude: &HashSet<String>) -> Vec<Hit> {
    let Some(q) = fts_query(question) else { return Vec::new() };
    let rows: Vec<(String, String, String, String, String, Option<f64>)> = sqlx::query_as(
        "SELECT s.meeting_id, m.title, m.created_at, s.source, s.content, t.audio_start_time
         FROM search_index s JOIN meetings m ON m.id = s.meeting_id
         LEFT JOIN transcripts t ON s.source = 'transcript' AND t.id = s.ref_id
         WHERE search_index MATCH ?1 ORDER BY bm25(search_index) LIMIT 40",
    )
    .bind(q)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .filter(|r| !exclude.contains(&r.0) && project_meetings.is_none_or(|p| p.contains(&r.0)))
        .map(|(meeting_id, title, created, source, content, start)| Hit {
            meeting_id,
            meeting_title: title,
            meeting_date: created.chars().take(10).collect(),
            text: if source == "summary" {
                hearken_core::export::summary_markdown(&content).unwrap_or(content)
            } else {
                content
            },
            source: if source == "action" || source == "decision" { "item".into() } else { source },
            start_seconds: start,
            score: 0.0,
        })
        .collect()
}

/// Merges meaning and keyword hits, alternating so both kinds are represented, without duplicates.
fn merge(meaning: Vec<Hit>, keyword: Vec<Hit>) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    let mut seen = HashSet::new();
    let (mut a, mut b) = (meaning.into_iter(), keyword.into_iter());
    loop {
        let (x, y) = (a.next(), b.next());
        if x.is_none() && y.is_none() {
            break;
        }
        for h in [x, y].into_iter().flatten() {
            let key = (h.meeting_id.clone(), h.text.chars().take(80).collect::<String>());
            if seen.insert(key) {
                out.push(h);
            }
        }
        if out.len() >= MAX_EXCERPTS {
            break;
        }
    }
    out.truncate(MAX_EXCERPTS);
    out
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_EXCERPT_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_EXCERPT_CHARS).collect();
    format!("{}…", cut.rsplit_once(' ').map(|(a, _)| a).unwrap_or(&cut))
}

fn build_prompt(question: &str, excerpts: &[Excerpt], today: &str) -> String {
    let list = excerpts
        .iter()
        .map(|e| {
            let at = e.start_seconds.map(|s| format!(" at [{}]", format_clock(s))).unwrap_or_default();
            format!("[{}] \"{}\" ({}){}, {}:\n{}", e.n, e.meeting_title, e.meeting_date, at, e.source, e.text)
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "Today is {today}.\n\nExcerpts from the user's meetings, lectures and notes:\n\n{list}\n\n\
         Question: {question}\n\n\
         Answer using only these excerpts, in a few sentences or short bullets. Cite every statement with its \
         excerpt number like [3]. Mention the meeting and date when it helps. If the excerpts do not answer the \
         question, reply with exactly {NOT_FOUND} and nothing else."
    )
}

/// Citation numbers in the answer that point at a real excerpt.
pub fn cited_numbers(answer: &str, count: usize) -> HashSet<usize> {
    let mut out = HashSet::new();
    let bytes: Vec<char> = answer.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '[' {
            let mut j = i + 1;
            let mut num = String::new();
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == ',' || bytes[j] == ' ') {
                num.push(bytes[j]);
                j += 1;
            }
            if j < bytes.len() && bytes[j] == ']' {
                for part in num.split(',') {
                    if let Ok(n) = part.trim().parse::<usize>() {
                        if (1..=count).contains(&n) {
                            out.insert(n);
                        }
                    }
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[tauri::command]
pub async fn api_ask<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    question: String,
    project_id: Option<String>,
) -> Result<Answer, String> {
    let pool = state.db_manager.pool();
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("Ask a question".into());
    }
    let model = current_model(&app, pool).await?;
    // A cloud model never sees local-only meetings.
    let exclude = if model.is_local() { HashSet::new() } else { local_only_meetings(pool).await };
    let project_meetings: Option<HashSet<String>> = match &project_id {
        Some(pid) => Some(
            sqlx::query_scalar::<_, String>("SELECT meeting_id FROM meeting_projects WHERE project_id = ?")
                .bind(pid)
                .fetch_all(pool)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .collect(),
        ),
        None => None,
    };

    let settings = load_settings(&app, pool).await;
    let mut meaning = Vec::new();
    if settings.semantic_search {
        match embeddings::search(pool, &settings, &question, 30, &exclude).await {
            Ok(hits) => meaning = hits.into_iter().filter(|h| project_meetings.as_ref().is_none_or(|p| p.contains(&h.meeting_id))).take(10).collect(),
            Err(e) => embeddings::warn_once(&e),
        }
    }
    let keyword = keyword_hits(pool, &question, project_meetings.as_ref(), &exclude).await;
    let excerpts: Vec<Excerpt> = merge(meaning.clone(), keyword)
        .into_iter()
        .enumerate()
        .map(|(i, h)| Excerpt {
            n: i + 1,
            meeting_id: h.meeting_id,
            meeting_title: h.meeting_title,
            meeting_date: h.meeting_date,
            source: h.source,
            start_seconds: h.start_seconds,
            text: truncate(&h.text),
            cited: false,
        })
        .collect();

    let not_found = |excerpts: Vec<Excerpt>| Answer {
        found: false,
        answer: "I couldn’t find that in your meetings.".into(),
        excerpts,
        model: model.label(),
        used_meaning_search: settings.semantic_search,
    };
    if excerpts.is_empty() {
        return Ok(not_found(Vec::new()));
    }

    let today = chrono::Local::now().format("%A %-d %B %Y").to_string();
    let raw = model
        .complete(
            "You answer questions about the user's own meetings using only the numbered excerpts you are given. \
             You never guess names, dates or numbers, and you cite excerpts like [2].",
            &build_prompt(&question, &excerpts, &today),
        )
        .await?;
    let answer = crate::summary::processor::clean_llm_markdown_detailed(&raw).markdown.trim().to_string();
    let cited = cited_numbers(&answer, excerpts.len());
    if answer.contains(NOT_FOUND) || answer.is_empty() || cited.is_empty() {
        // An uncited answer isn't grounded in the excerpts, so it isn't shown as an answer.
        return Ok(not_found(excerpts));
    }
    let excerpts = excerpts.into_iter().map(|e| Excerpt { cited: cited.contains(&e.n), ..e }).collect();
    Ok(Answer { found: true, answer, excerpts, model: model.label(), used_meaning_search: settings.semantic_search })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_query_drops_filler() {
        assert_eq!(fts_query("What did we decide about the budget forecast?").unwrap(), "\"decide\"* OR \"budget\"* OR \"forecast\"*");
        assert!(fts_query("what is it?").is_none());
    }

    #[test]
    fn only_real_citations_count() {
        let c = cited_numbers("Kofi owns it [1]. Due Friday [2, 3]. Also [9] and [x].", 3);
        assert_eq!(c, HashSet::from([1, 2, 3]));
        assert!(cited_numbers("No citations here.", 3).is_empty());
    }

    fn hit(id: &str, text: &str) -> Hit {
        Hit { meeting_id: id.into(), meeting_title: "T".into(), meeting_date: "2026-10-14".into(), source: "transcript".into(), start_seconds: None, text: text.into(), score: 0.0 }
    }

    #[test]
    fn merge_alternates_and_dedupes() {
        let m = merge(vec![hit("a", "one"), hit("b", "two")], vec![hit("a", "one"), hit("c", "three")]);
        assert_eq!(m.iter().map(|h| h.meeting_id.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c"]);
    }

    #[test]
    fn long_excerpts_are_cut_at_a_word() {
        let t = truncate(&"word ".repeat(300));
        assert!(t.ends_with('…') && t.chars().count() <= MAX_EXCERPT_CHARS + 1);
    }
}

#[cfg(test)]
mod live_tests {
    //! Run with HEARKEN_TEST_OLLAMA=1 and Ollama running (nomic-embed-text and qwen2.5:14b pulled).
    use super::*;
    use crate::intelligence::{embeddings, CurrentModel, IntelligenceSettings};
    use crate::summary::llm_client::LLMProvider;
    use crate::summary::llm_config::ResolvedLlm;

    #[tokio::test]
    async fn meaning_search_and_grounded_answers() {
        if std::env::var("HEARKEN_TEST_OLLAMA").is_err() {
            return;
        }
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1', 'Finance sync', '2026-10-14T10:00:00Z', 'x')")
            .execute(&pool).await.unwrap();
        for (i, (t, at)) in [
            ("Good morning everyone, thanks for joining.", 2.0),
            ("Kofi will send the revised budget forecast for 2027 by Friday.", 754.0),
            ("We agreed to pause the Accra office lease renewal until January.", 1200.0),
            ("The new hires start in March, onboarding is with Ama.", 1500.0),
        ].iter().enumerate() {
            sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time) VALUES (?, 'm1', ?, 'x', ?)")
                .bind(format!("t{i}")).bind(t).bind(at).execute(&pool).await.unwrap();
        }
        let settings = IntelligenceSettings { semantic_search: true, embed_model: "nomic-embed-text".into(), ollama_endpoint: "http://localhost:11434".into() };
        assert!(embeddings::index_meeting(&pool, &settings, "m1").await.unwrap() > 0);
        assert_eq!(embeddings::index_meeting(&pool, &settings, "m1").await.unwrap(), 0, "unchanged meetings aren't re-embedded");

        let hits = embeddings::search(&pool, &settings, "money plan for next year", 3, &Default::default()).await.unwrap();
        println!("meaning search: {:?}", hits.iter().map(|h| (h.score, &h.text)).collect::<Vec<_>>());
        assert!(hits[0].text.contains("budget forecast"), "found by meaning, without the words");

        let model = CurrentModel {
            provider: LLMProvider::Ollama,
            model: "qwen2.5:14b".into(),
            llm: ResolvedLlm { api_key: String::new(), ollama_endpoint: None, custom_openai_endpoint: None, max_tokens: None, temperature: None, top_p: None, token_threshold: 4000, app_data_dir: None },
        };
        let excerpts: Vec<Excerpt> = hits.into_iter().enumerate().map(|(i, h)| Excerpt {
            n: i + 1, meeting_id: h.meeting_id, meeting_title: h.meeting_title, meeting_date: h.meeting_date,
            source: h.source, start_seconds: h.start_seconds, text: h.text, cited: false,
        }).collect();
        let answer = model.complete("You answer using only the numbered excerpts and cite them like [2].",
            &build_prompt("Who is sending the budget forecast and when?", &excerpts, "Thursday 15 October 2026")).await.unwrap();
        println!("answer: {answer}");
        assert!(answer.contains("Kofi") && !cited_numbers(&answer, excerpts.len()).is_empty());
        let unknown = model.complete("You answer using only the numbered excerpts and cite them like [2].",
            &build_prompt("What colour is the company logo?", &excerpts, "Thursday 15 October 2026")).await.unwrap();
        println!("unknown: {unknown}");
        assert!(unknown.contains(NOT_FOUND) || cited_numbers(&unknown, excerpts.len()).is_empty(), "no grounded answer is invented");
    }
}
