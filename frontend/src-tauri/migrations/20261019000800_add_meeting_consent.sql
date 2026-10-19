-- When the user confirmed they told everyone they were recording (from the prompt at start).
CREATE TABLE IF NOT EXISTS meeting_consent (
    meeting_id TEXT PRIMARY KEY,
    confirmed_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);
