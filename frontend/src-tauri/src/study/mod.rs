//! Study aids from lectures: flashcards, practice quiz questions and glossary terms,
//! each grounded in a transcript quote (checked with hearken-core, like action items);
//! spaced-repetition review; Anki export; and a per-course glossary.

use crate::intelligence::current_model;
use crate::state::AppState;
use crate::summary::processor::{chunk_text, rough_token_count};
use chrono::{Duration, Local, Utc};
use hearken_core::items::{load_timed_transcript, locate_quote, normalize, render_transcript};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use tauri::{AppHandle, Runtime};
use uuid::Uuid;

const MAX_PASS_TOKENS: usize = 6000;

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct StudyCard {
    pub id: String,
    pub meeting_id: String,
    pub kind: String,
    pub front: String,
    pub back: String,
    pub choices: Option<String>,
    pub answer_index: Option<i64>,
    pub evidence_quote: Option<String>,
    pub start_seconds: Option<f64>,
    pub verified: bool,
    pub source: String,
    pub due_on: Option<String>,
    pub interval_days: f64,
    pub ease: f64,
    pub reps: i64,
    pub lapses: i64,
    pub last_reviewed: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Default, Deserialize)]
struct Generated {
    #[serde(default)]
    flashcards: Vec<RawCard>,
    #[serde(default)]
    quiz: Vec<RawQuiz>,
    #[serde(default)]
    terms: Vec<RawTerm>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCard {
    #[serde(default)]
    question: String,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    evidence_quote: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawQuiz {
    #[serde(default)]
    question: String,
    #[serde(default)]
    choices: Vec<String>,
    #[serde(default)]
    answer_index: Option<i64>,
    #[serde(default)]
    explanation: String,
    #[serde(default)]
    evidence_quote: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTerm {
    #[serde(default)]
    term: String,
    #[serde(default)]
    definition: String,
    #[serde(default)]
    evidence_quote: Option<String>,
}

fn build_prompt(chunk: &str) -> String {
    format!(
        r#"Read this part of a lecture transcript. Each line starts with its time in the recording.

Create study material using ONLY what the lecturer said here:
1. flashcards: up to 12 question/answer pairs that test the key facts, concepts and steps. Short answers.
2. quiz: up to 6 multiple-choice questions, each with exactly 4 choices, exactly one correct (answer_index 0-3), plausible wrong choices, and a one-sentence explanation.
3. terms: every term the lecturer defined or explained, with the definition as they gave it.

For every item, "evidence_quote" must be 5 to 25 consecutive words copied EXACTLY from one transcript line that supports it, without the [time] prefix.
Do not add facts the lecturer didn't say. If there is nothing suitable, return empty arrays.

Return only this JSON:
{{"flashcards":[{{"question":"","answer":"","evidence_quote":""}}],"quiz":[{{"question":"","choices":["","","",""],"answer_index":0,"explanation":"","evidence_quote":""}}],"terms":[{{"term":"","definition":"","evidence_quote":""}}]}}

<transcript>
{chunk}
</transcript>"#
    )
}

fn parse(raw: &str) -> Option<Generated> {
    let (start, end) = (raw.find('{')?, raw.rfind('}')?);
    (end > start).then(|| serde_json::from_str(&raw[start..=end]).ok()).flatten()
}

/// Turns model output into cards: drops empty/duplicate/malformed ones and grounds quotes.
fn ground(meeting_id: &str, generated: Generated, lines: &[hearken_core::items::TimedLine]) -> Vec<StudyCard> {
    let now = Utc::now().to_rfc3339();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut push = |kind: &str, front: String, back: String, choices: Option<String>, answer: Option<i64>, quote: Option<String>| {
        let (front, back) = (front.trim().to_string(), back.trim().to_string());
        if front.is_empty() || back.is_empty() || !seen.insert((kind.to_string(), normalize(&front))) {
            return;
        }
        let quote = quote.map(|q| q.trim().to_string()).filter(|q| !q.is_empty());
        let (verified, at) = quote.as_deref().map(|q| locate_quote(lines, q)).unwrap_or((false, None));
        out.push(StudyCard {
            id: Uuid::new_v4().to_string(),
            meeting_id: meeting_id.to_string(),
            kind: kind.to_string(),
            front,
            back,
            choices,
            answer_index: answer,
            evidence_quote: quote,
            start_seconds: at,
            verified,
            source: "ai".into(),
            due_on: None,
            interval_days: 0.0,
            ease: 2.5,
            reps: 0,
            lapses: 0,
            last_reviewed: None,
            created_at: now.clone(),
        });
    };
    for c in generated.flashcards {
        push("flashcard", c.question, c.answer, None, None, c.evidence_quote);
    }
    for q in generated.quiz {
        let choices: Vec<String> = q.choices.into_iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
        let valid = choices.len() == 4 && q.answer_index.is_some_and(|i| (0..4).contains(&i));
        if valid {
            push("quiz", q.question, q.explanation, Some(serde_json::to_string(&choices).unwrap_or_default()), q.answer_index, q.evidence_quote);
        }
    }
    for t in generated.terms {
        push("term", t.term, t.definition, None, None, t.evidence_quote);
    }
    out
}

async fn insert(pool: &SqlitePool, c: &StudyCard) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO study_cards (id, meeting_id, kind, front, back, choices, answer_index, evidence_quote, start_seconds, verified, source, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&c.id)
    .bind(&c.meeting_id)
    .bind(&c.kind)
    .bind(&c.front)
    .bind(&c.back)
    .bind(&c.choices)
    .bind(c.answer_index)
    .bind(&c.evidence_quote)
    .bind(c.start_seconds)
    .bind(c.verified)
    .bind(&c.source)
    .bind(&c.created_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Spaced repetition (a light SM-2): rating 1 = again, 2 = hard, 3 = good, 4 = easy.
/// Returns (interval_days, ease, reps, lapses, due in days).
pub fn schedule(interval: f64, ease: f64, reps: i64, lapses: i64, rating: u8) -> (f64, f64, i64, i64, i64) {
    match rating {
        1 => (0.0, (ease - 0.2).max(1.3), 0, lapses + 1, 0),
        2 => {
            let i = (interval * 1.2).max(1.0);
            (i, (ease - 0.15).max(1.3), reps + 1, lapses, i.round() as i64)
        }
        _ => {
            let good = match reps {
                0 => 1.0,
                1 => 3.0,
                _ => (interval * ease).max(interval + 1.0),
            };
            let (i, e) = if rating >= 4 { (good * 1.3, ease + 0.15) } else { (good, ease) };
            (i, e, reps + 1, lapses, i.round() as i64)
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateResult {
    pub flashcards: usize,
    pub quiz: usize,
    pub terms: usize,
    pub unverified: usize,
}

/// Makes study aids for one lecture, replacing its previous AI-made ones (cards you
/// added yourself are kept).
#[tauri::command]
pub async fn api_generate_study_aids<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<GenerateResult, String> {
    let pool = state.db_manager.pool();
    let model = current_model(&app, pool).await?;
    if !model.is_local() && crate::projects::is_local_only(pool, &meeting_id).await {
        return Err("This lecture is in a local-only course, so it can't be sent to a cloud model".into());
    }
    let lines = load_timed_transcript(pool, &meeting_id).await.map_err(|e| e.to_string())?;
    if lines.is_empty() {
        return Err("This recording has no transcript yet".into());
    }
    let transcript = render_transcript(&lines);
    let pass = model.llm.token_threshold.saturating_sub(800).clamp(1000, MAX_PASS_TOKENS);
    let chunks = if rough_token_count(&transcript) <= pass { vec![transcript] } else { chunk_text(&transcript, pass, 100) };

    let mut all = Generated::default();
    for chunk in &chunks {
        let raw = model
            .complete("You write study material from lecture transcripts. You output only valid JSON and never invent facts.", &build_prompt(chunk))
            .await?;
        match parse(&raw) {
            Some(g) => {
                all.flashcards.extend(g.flashcards);
                all.quiz.extend(g.quiz);
                all.terms.extend(g.terms);
            }
            None => log::warn!("Study aids pass returned no usable JSON for {}", meeting_id),
        }
    }
    let cards = ground(&meeting_id, all, &lines);
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM study_cards WHERE meeting_id = ? AND source = 'ai'").bind(&meeting_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    for c in &cards {
        insert(pool, c).await?;
    }
    let count = |k: &str| cards.iter().filter(|c| c.kind == k).count();
    Ok(GenerateResult {
        flashcards: count("flashcard"),
        quiz: count("quiz"),
        terms: count("term"),
        unverified: cards.iter().filter(|c| !c.verified).count(),
    })
}

/// Cards for one lecture, or (course_id) for every lecture in a course.
#[tauri::command]
pub async fn api_list_study_cards(
    state: tauri::State<'_, AppState>,
    meeting_id: Option<String>,
    course_id: Option<String>,
    kind: Option<String>,
) -> Result<Vec<StudyCard>, String> {
    sqlx::query_as::<_, StudyCard>(
        "SELECT c.* FROM study_cards c
         WHERE (?1 IS NULL OR c.meeting_id = ?1)
           AND (?2 IS NULL OR c.meeting_id IN (SELECT meeting_id FROM meeting_projects WHERE project_id = ?2))
           AND (?3 IS NULL OR c.kind = ?3)
         ORDER BY c.kind, COALESCE(c.start_seconds, 1e12), c.created_at",
    )
    .bind(meeting_id)
    .bind(course_id)
    .bind(kind)
    .fetch_all(state.db_manager.pool())
    .await
    .map_err(|e| e.to_string())
}

/// Flashcards and terms due for review today (new ones first), for a lecture or course.
#[tauri::command]
pub async fn api_due_cards(
    state: tauri::State<'_, AppState>,
    meeting_id: Option<String>,
    course_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<StudyCard>, String> {
    let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
    sqlx::query_as::<_, StudyCard>(
        "SELECT c.* FROM study_cards c
         WHERE c.kind IN ('flashcard', 'term') AND (c.due_on IS NULL OR c.due_on <= ?3)
           AND (?1 IS NULL OR c.meeting_id = ?1)
           AND (?2 IS NULL OR c.meeting_id IN (SELECT meeting_id FROM meeting_projects WHERE project_id = ?2))
         ORDER BY c.due_on IS NOT NULL, c.due_on, c.created_at LIMIT ?4",
    )
    .bind(meeting_id)
    .bind(course_id)
    .bind(today)
    .bind(limit.unwrap_or(50).clamp(1, 500))
    .fetch_all(state.db_manager.pool())
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_review_card(state: tauri::State<'_, AppState>, id: String, rating: u8) -> Result<StudyCard, String> {
    if !(1..=4).contains(&rating) {
        return Err("Rating must be 1 (again) to 4 (easy)".into());
    }
    let pool = state.db_manager.pool();
    let card = sqlx::query_as::<_, StudyCard>("SELECT * FROM study_cards WHERE id = ?")
        .bind(&id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Card not found")?;
    let (interval, ease, reps, lapses, due_in) = schedule(card.interval_days, card.ease, card.reps, card.lapses, rating);
    let due = (Local::now().date_naive() + Duration::days(due_in)).format("%Y-%m-%d").to_string();
    sqlx::query("UPDATE study_cards SET interval_days = ?, ease = ?, reps = ?, lapses = ?, due_on = ?, last_reviewed = ? WHERE id = ?")
        .bind(interval)
        .bind(ease)
        .bind(reps)
        .bind(lapses)
        .bind(&due)
        .bind(Utc::now().to_rfc3339())
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query_as::<_, StudyCard>("SELECT * FROM study_cards WHERE id = ?").bind(&id).fetch_one(pool).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_add_study_card(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    kind: String,
    front: String,
    back: String,
) -> Result<StudyCard, String> {
    if !matches!(kind.as_str(), "flashcard" | "term") {
        return Err("You can add flashcards and terms".into());
    }
    if front.trim().is_empty() || back.trim().is_empty() {
        return Err("Fill in both sides".into());
    }
    let card = StudyCard {
        id: Uuid::new_v4().to_string(),
        meeting_id,
        kind,
        front: front.trim().into(),
        back: back.trim().into(),
        choices: None,
        answer_index: None,
        evidence_quote: None,
        start_seconds: None,
        verified: false,
        source: "user".into(),
        due_on: None,
        interval_days: 0.0,
        ease: 2.5,
        reps: 0,
        lapses: 0,
        last_reviewed: None,
        created_at: Utc::now().to_rfc3339(),
    };
    insert(state.db_manager.pool(), &card).await?;
    Ok(card)
}

#[tauri::command]
pub async fn api_delete_study_card(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    sqlx::query("DELETE FROM study_cards WHERE id = ?").bind(&id).execute(state.db_manager.pool()).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Anki-importable text: front<TAB>back<TAB>tags, one card per line.
pub fn anki_tsv(cards: &[(String, String, String)]) -> String {
    let clean = |s: &str| s.replace(['\t', '\r'], " ").replace('\n', "<br>");
    let mut out = String::from("#separator:tab\n#html:true\n#tags column:3\n");
    for (front, back, tags) in cards {
        out.push_str(&format!("{}\t{}\t{}\n", clean(front), clean(back), tags.replace(char::is_whitespace, "_")));
    }
    out
}

/// Saves flashcards and terms (for a lecture or a whole course) as a file Anki imports.
#[tauri::command]
pub async fn api_export_anki<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: Option<String>,
    course_id: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let pool = state.db_manager.pool();
    let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT c.front, c.back, m.title,
            (SELECT p.name FROM meeting_projects mp JOIN projects p ON p.id = mp.project_id WHERE mp.meeting_id = m.id AND mp.is_primary = 1)
         FROM study_cards c JOIN meetings m ON m.id = c.meeting_id
         WHERE c.kind IN ('flashcard', 'term')
           AND (?1 IS NULL OR c.meeting_id = ?1)
           AND (?2 IS NULL OR c.meeting_id IN (SELECT meeting_id FROM meeting_projects WHERE project_id = ?2))
         ORDER BY m.created_at, c.created_at",
    )
    .bind(&meeting_id)
    .bind(&course_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Err("No flashcards or terms to export yet".into());
    }
    let cards: Vec<(String, String, String)> = rows
        .into_iter()
        .map(|(f, b, lecture, course)| (f, b, format!("hearken {} {}", course.unwrap_or_default(), lecture).trim().to_string()))
        .collect();
    let name = format!("Hearken flashcards ({} cards).txt", cards.len());
    let app2 = app.clone();
    let picked = tokio::task::spawn_blocking(move || app2.dialog().file().set_file_name(&name).add_filter("Anki import (text)", &["txt"]).blocking_save_file())
        .await
        .map_err(|e| e.to_string())?;
    let Some(p) = picked else { return Ok(None) };
    let path = p.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, anki_tsv(&cards)).map_err(|e| e.to_string())?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hearken_core::items::TimedLine;

    fn lines() -> Vec<TimedLine> {
        vec![
            TimedLine { start_seconds: Some(60.0), text: "Remote sensing is the collection of data about an object without touching it.".into(), speaker: None },
            TimedLine { start_seconds: Some(300.0), text: "NDVI uses the red and near infrared bands to measure vegetation health.".into(), speaker: None },
        ]
    }

    #[test]
    fn grounds_and_filters_generated_material() {
        let raw = r#"Here: {"flashcards":[{"question":"What does NDVI measure?","answer":"Vegetation health","evidence_quote":"NDVI uses the red and near infrared bands"},
            {"question":"what does ndvi measure?","answer":"dup","evidence_quote":""},
            {"question":"Invented?","answer":"Yes","evidence_quote":"the lecturer never said this sentence at all"}],
          "quiz":[{"question":"Which bands does NDVI use?","choices":["Red and NIR","Blue","Green","Thermal"],"answer_index":0,"explanation":"Red and near infrared.","evidence_quote":"red and near infrared bands"},
                  {"question":"Broken","choices":["a","b"],"answer_index":0,"explanation":"x"}],
          "terms":[{"term":"Remote sensing","definition":"Collecting data without touching the object","evidence_quote":"collection of data about an object without touching it"}]}"#;
        let cards = ground("m1", parse(raw).unwrap(), &lines());
        let fronts: Vec<&str> = cards.iter().map(|c| c.front.as_str()).collect();
        assert_eq!(fronts, vec!["What does NDVI measure?", "Invented?", "Which bands does NDVI use?", "Remote sensing"], "duplicate and malformed quiz dropped");
        assert!(cards[0].verified && cards[0].start_seconds == Some(300.0));
        assert!(!cards[1].verified && cards[1].start_seconds.is_none(), "an invented quote is flagged, with no time");
        assert_eq!(cards[2].answer_index, Some(0));
        assert_eq!(cards[3].start_seconds, Some(60.0));
    }

    #[test]
    fn spaced_repetition_grows_intervals_and_resets_on_again() {
        let (i1, e1, r1, _, d1) = schedule(0.0, 2.5, 0, 0, 3);
        assert_eq!((i1, r1, d1), (1.0, 1, 1));
        let (i2, _, r2, _, d2) = schedule(i1, e1, r1, 0, 3);
        assert_eq!((i2, r2, d2), (3.0, 2, 3));
        let (i3, _, _, _, d3) = schedule(i2, e1, r2, 0, 3);
        assert_eq!((i3, d3), (7.5, 8));
        let (ia, ea, ra, la, da) = schedule(i3, e1, 3, 0, 1);
        assert_eq!((ia, ra, la, da), (0.0, 0, 1, 0));
        assert!((ea - 2.3).abs() < 1e-9);
        let (ie, ee, _, _, _) = schedule(0.0, 2.5, 0, 0, 4);
        assert!((ie - 1.3).abs() < 1e-9 && (ee - 2.65).abs() < 1e-9);
    }

    #[test]
    fn anki_export_is_tab_separated_with_tags() {
        let tsv = anki_tsv(&[("Q\twith tab".into(), "line1\nline2".into(), "hearken GEOG 301 Lecture 3".into())]);
        assert!(tsv.starts_with("#separator:tab\n#html:true\n#tags column:3\n"));
        assert!(tsv.ends_with("Q with tab\tline1<br>line2\thearken_GEOG_301_Lecture_3\n"));
    }
}

#[cfg(test)]
mod live_tests {
    //! HEARKEN_TEST_OLLAMA=1 with Ollama running and qwen2.5:14b pulled.
    use super::*;
    use crate::intelligence::CurrentModel;
    use crate::summary::llm_client::LLMProvider;
    use crate::summary::llm_config::ResolvedLlm;
    use hearken_core::items::TimedLine;

    #[tokio::test]
    async fn qwen_makes_grounded_study_aids() {
        if std::env::var("HEARKEN_TEST_OLLAMA").is_err() {
            return;
        }
        let lines: Vec<TimedLine> = [
            (30.0, "Today we start remote sensing. Remote sensing is the collection of data about an object without touching it."),
            (95.0, "Satellites carry sensors that record reflected light in different bands of the spectrum."),
            (180.0, "NDVI, the normalised difference vegetation index, uses the red and near infrared bands to measure vegetation health."),
            (260.0, "Healthy plants reflect a lot of near infrared light and absorb red light, so their NDVI is high."),
            (340.0, "This will definitely be in the exam: NDVI ranges from minus one to plus one."),
            (420.0, "For next week, read chapter four and submit the lab report by Friday."),
        ].iter().map(|(t, s)| TimedLine { start_seconds: Some(*t), text: s.to_string(), speaker: None }).collect();
        let model = CurrentModel {
            provider: LLMProvider::Ollama,
            model: "qwen2.5:14b".into(),
            llm: ResolvedLlm { api_key: String::new(), ollama_endpoint: None, custom_openai_endpoint: None, max_tokens: None, temperature: None, top_p: None, token_threshold: 8000, app_data_dir: None },
        };
        let raw = model.complete("You write study material from lecture transcripts. You output only valid JSON and never invent facts.",
            &build_prompt(&render_transcript(&lines))).await.unwrap();
        let cards = ground("m1", parse(&raw).expect("valid JSON"), &lines);
        for c in &cards {
            println!("{:9} verified={} at={:?} | {} => {}", c.kind, c.verified, c.start_seconds, c.front, c.back);
        }
        assert!(cards.iter().any(|c| c.kind == "flashcard"));
        assert!(cards.iter().any(|c| c.kind == "quiz"));
        assert!(cards.iter().any(|c| c.kind == "term"));
        let verified = cards.iter().filter(|c| c.verified).count();
        println!("{verified}/{} verified", cards.len());
        assert!(verified * 2 >= cards.len(), "most quotes are found in the transcript");
    }
}
