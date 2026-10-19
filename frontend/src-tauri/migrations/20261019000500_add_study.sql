-- Study aids made from lectures: flashcards, quiz questions and glossary terms. Each one
-- carries the transcript quote it came from (verified like action items) so the user can
-- jump to the moment the lecturer explained it.
CREATE TABLE IF NOT EXISTS study_cards (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('flashcard', 'quiz', 'term')),
    front TEXT NOT NULL,             -- question, quiz question or term
    back TEXT NOT NULL,              -- answer, explanation or definition
    choices TEXT,                    -- quiz: JSON array of 4 options
    answer_index INTEGER,            -- quiz: which option is right
    evidence_quote TEXT,
    start_seconds REAL,
    verified INTEGER NOT NULL DEFAULT 0,
    source TEXT NOT NULL DEFAULT 'ai' CHECK (source IN ('ai', 'user')),
    -- spaced repetition (flashcards and terms)
    due_on TEXT,                     -- YYYY-MM-DD; NULL = new
    interval_days REAL NOT NULL DEFAULT 0,
    ease REAL NOT NULL DEFAULT 2.5,
    reps INTEGER NOT NULL DEFAULT 0,
    lapses INTEGER NOT NULL DEFAULT 0,
    last_reviewed TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_study_cards_meeting ON study_cards(meeting_id, kind);
CREATE INDEX IF NOT EXISTS idx_study_cards_due ON study_cards(kind, due_on);
