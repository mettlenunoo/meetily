-- Full-text search across all meetings: titles, transcript segments,
-- AI summaries and the user's own notes. Kept in sync by triggers.
--
-- source:  'title' | 'transcript' | 'summary' | 'notes'
-- ref_id:  meetings.id, transcripts.id, or the meeting_id (summary/notes)
-- ts:      transcript segment timestamp, so a hit can jump to that moment
CREATE VIRTUAL TABLE IF NOT EXISTS search_index USING fts5(
    content,
    meeting_id UNINDEXED,
    source UNINDEXED,
    ref_id UNINDEXED,
    ts UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Backfill existing data
INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    SELECT title, id, 'title', id, created_at FROM meetings;

INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    SELECT transcript, meeting_id, 'transcript', id, timestamp FROM transcripts;

INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    SELECT result, meeting_id, 'summary', meeting_id, updated_at
    FROM summary_processes WHERE result IS NOT NULL AND result != '';

INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    SELECT notes_markdown, meeting_id, 'notes', meeting_id, updated_at
    FROM meeting_notes WHERE notes_markdown IS NOT NULL AND notes_markdown != '';

-- Meetings (titles)
CREATE TRIGGER IF NOT EXISTS search_meetings_ai AFTER INSERT ON meetings BEGIN
    INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    VALUES (new.title, new.id, 'title', new.id, new.created_at);
END;

CREATE TRIGGER IF NOT EXISTS search_meetings_au AFTER UPDATE OF title ON meetings BEGIN
    DELETE FROM search_index WHERE source = 'title' AND ref_id = old.id;
    INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    VALUES (new.title, new.id, 'title', new.id, new.created_at);
END;

-- Deleting a meeting removes everything indexed for it
-- (child rows are removed by ON DELETE CASCADE, which may not fire child triggers).
CREATE TRIGGER IF NOT EXISTS search_meetings_ad AFTER DELETE ON meetings BEGIN
    DELETE FROM search_index WHERE meeting_id = old.id;
END;

-- Transcript segments
CREATE TRIGGER IF NOT EXISTS search_transcripts_ai AFTER INSERT ON transcripts BEGIN
    INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    VALUES (new.transcript, new.meeting_id, 'transcript', new.id, new.timestamp);
END;

CREATE TRIGGER IF NOT EXISTS search_transcripts_au AFTER UPDATE OF transcript ON transcripts BEGIN
    DELETE FROM search_index WHERE source = 'transcript' AND ref_id = old.id;
    INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    VALUES (new.transcript, new.meeting_id, 'transcript', new.id, new.timestamp);
END;

CREATE TRIGGER IF NOT EXISTS search_transcripts_ad AFTER DELETE ON transcripts BEGIN
    DELETE FROM search_index WHERE source = 'transcript' AND ref_id = old.id;
END;

-- AI summaries
CREATE TRIGGER IF NOT EXISTS search_summary_ai AFTER INSERT ON summary_processes
WHEN new.result IS NOT NULL AND new.result != '' BEGIN
    INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    VALUES (new.result, new.meeting_id, 'summary', new.meeting_id, new.updated_at);
END;

CREATE TRIGGER IF NOT EXISTS search_summary_au AFTER UPDATE OF result ON summary_processes BEGIN
    DELETE FROM search_index WHERE source = 'summary' AND meeting_id = old.meeting_id;
    INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    SELECT new.result, new.meeting_id, 'summary', new.meeting_id, new.updated_at
    WHERE new.result IS NOT NULL AND new.result != '';
END;

CREATE TRIGGER IF NOT EXISTS search_summary_ad AFTER DELETE ON summary_processes BEGIN
    DELETE FROM search_index WHERE source = 'summary' AND meeting_id = old.meeting_id;
END;

-- User notes
CREATE TRIGGER IF NOT EXISTS search_notes_ai AFTER INSERT ON meeting_notes
WHEN new.notes_markdown IS NOT NULL AND new.notes_markdown != '' BEGIN
    INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    VALUES (new.notes_markdown, new.meeting_id, 'notes', new.meeting_id, new.updated_at);
END;

CREATE TRIGGER IF NOT EXISTS search_notes_au AFTER UPDATE OF notes_markdown ON meeting_notes BEGIN
    DELETE FROM search_index WHERE source = 'notes' AND meeting_id = old.meeting_id;
    INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    SELECT new.notes_markdown, new.meeting_id, 'notes', new.meeting_id, new.updated_at
    WHERE new.notes_markdown IS NOT NULL AND new.notes_markdown != '';
END;

CREATE TRIGGER IF NOT EXISTS search_notes_ad AFTER DELETE ON meeting_notes BEGIN
    DELETE FROM search_index WHERE source = 'notes' AND meeting_id = old.meeting_id;
END;
