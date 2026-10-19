-- What kind of recording this is: it picks the summary template and what gets extracted.
ALTER TABLE meetings ADD COLUMN recording_type TEXT NOT NULL DEFAULT 'meeting'
    CHECK (recording_type IN ('meeting', 'lecture', 'interview', 'voice_note'));

-- Moments marked as important, while recording (hotkey / button) or later in the player.
CREATE TABLE IF NOT EXISTS meeting_bookmarks (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    seconds REAL NOT NULL,      -- from the start of the recording, like transcripts.audio_start_time
    note TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_meeting_bookmarks_meeting ON meeting_bookmarks(meeting_id, seconds);
