-- Structured action items and decisions, each linked to the transcript
-- passage it came from so the user can verify it against the recording.
CREATE TABLE IF NOT EXISTS meeting_items (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('action', 'decision')),
    title TEXT NOT NULL,
    description TEXT,
    owner TEXT,                       -- NULL when the transcript doesn't say who
    due_date TEXT,                    -- as spoken ("Friday", "14 Oct"); NULL if not stated
    priority TEXT,                    -- 'high' | 'medium' | 'low' | NULL
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'done', 'dismissed')),
    evidence_quote TEXT,              -- verbatim transcript excerpt supporting the item
    evidence_start_time REAL,         -- seconds from recording start, for the audio player
    evidence_verified INTEGER NOT NULL DEFAULT 0, -- 1 = quote was found in the transcript
    source TEXT NOT NULL DEFAULT 'ai' CHECK (source IN ('ai', 'user')),
    user_edited INTEGER NOT NULL DEFAULT 0,       -- AI re-extraction never overwrites edited items
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_meeting_items_meeting ON meeting_items(meeting_id);
CREATE INDEX IF NOT EXISTS idx_meeting_items_status ON meeting_items(kind, status);

-- Make items searchable alongside transcripts, summaries and notes.
CREATE TRIGGER IF NOT EXISTS search_items_ai AFTER INSERT ON meeting_items BEGIN
    INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    VALUES (new.title || COALESCE(' — ' || new.owner, ''), new.meeting_id,
            CASE new.kind WHEN 'action' THEN 'action' ELSE 'decision' END, new.id, new.created_at);
END;

CREATE TRIGGER IF NOT EXISTS search_items_au AFTER UPDATE OF title, owner ON meeting_items BEGIN
    DELETE FROM search_index WHERE source IN ('action', 'decision') AND ref_id = old.id;
    INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    VALUES (new.title || COALESCE(' — ' || new.owner, ''), new.meeting_id,
            CASE new.kind WHEN 'action' THEN 'action' ELSE 'decision' END, new.id, new.created_at);
END;

CREATE TRIGGER IF NOT EXISTS search_items_ad AFTER DELETE ON meeting_items BEGIN
    DELETE FROM search_index WHERE source IN ('action', 'decision') AND ref_id = old.id;
END;
