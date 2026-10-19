-- Transcription model comparisons: the same clip transcribed by each model and scored
-- against the user's corrected text (word error rate), plus how fast each ran.
CREATE TABLE IF NOT EXISTS model_evals (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,            -- one comparison = one clip, several models
    meeting_id TEXT,
    from_seconds REAL NOT NULL,
    to_seconds REAL NOT NULL,
    engine TEXT NOT NULL,            -- 'whisper' | 'parakeet'
    model TEXT NOT NULL,
    reference_words INTEGER NOT NULL,
    errors INTEGER NOT NULL,         -- substitutions + deletions + insertions
    wer REAL NOT NULL,
    hypothesis TEXT NOT NULL,
    audio_seconds REAL NOT NULL,
    seconds_taken REAL NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_model_evals_model ON model_evals(engine, model);
