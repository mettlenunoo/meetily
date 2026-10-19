-- Search by meaning: passages of each meeting with a vector from a local embedding model.
CREATE TABLE IF NOT EXISTS embedding_chunks (
    id INTEGER PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    source TEXT NOT NULL,          -- transcript | summary | notes | item | document
    ref_id TEXT,
    start_seconds REAL,            -- transcript passages: where they start in the recording
    text TEXT NOT NULL,
    model TEXT NOT NULL,
    vector BLOB NOT NULL,          -- little-endian f32, normalised to length 1
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_embedding_chunks_model ON embedding_chunks(model, meeting_id);

-- What has been indexed, so unchanged meetings aren't embedded again.
CREATE TABLE IF NOT EXISTS embedding_status (
    meeting_id TEXT PRIMARY KEY,
    model TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    chunks INTEGER NOT NULL,
    indexed_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

-- A short, running "where things stand" for each project, refreshed after summaries.
ALTER TABLE projects ADD COLUMN brief TEXT;
ALTER TABLE projects ADD COLUMN brief_updated_at TEXT;

-- Small bits of app state (e.g. when the weekly digest notification last went out).
CREATE TABLE IF NOT EXISTS app_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
