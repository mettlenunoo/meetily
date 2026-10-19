-- Documents generated from a meeting (minutes, a formal report, lecture notes...) or a
-- project (status report). A meeting can have several, next to its summary.
CREATE TABLE IF NOT EXISTS documents (
    id TEXT PRIMARY KEY,
    meeting_id TEXT,
    project_id TEXT,
    template_id TEXT NOT NULL,
    title TEXT NOT NULL,
    markdown TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'ready' CHECK (status IN ('generating', 'ready', 'failed')),
    error TEXT,
    generated_by TEXT NOT NULL,     -- e.g. 'ollama:qwen2.5:14b', 'claude-via-mcp', 'user'
    user_edited INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK ((meeting_id IS NULL) != (project_id IS NULL)),
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_documents_meeting ON documents(meeting_id);
CREATE INDEX IF NOT EXISTS idx_documents_project ON documents(project_id);

-- Meeting documents are searchable like summaries and notes.
CREATE TRIGGER IF NOT EXISTS search_documents_ai AFTER INSERT ON documents
WHEN new.meeting_id IS NOT NULL AND new.status = 'ready' AND new.markdown != '' BEGIN
    INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    VALUES (new.title || ' ' || new.markdown, new.meeting_id, 'document', new.id, new.updated_at);
END;
CREATE TRIGGER IF NOT EXISTS search_documents_au AFTER UPDATE ON documents BEGIN
    DELETE FROM search_index WHERE source = 'document' AND ref_id = old.id;
    INSERT INTO search_index (content, meeting_id, source, ref_id, ts)
    SELECT new.title || ' ' || new.markdown, new.meeting_id, 'document', new.id, new.updated_at
    WHERE new.meeting_id IS NOT NULL AND new.status = 'ready' AND new.markdown != '';
END;
CREATE TRIGGER IF NOT EXISTS search_documents_ad AFTER DELETE ON documents BEGIN
    DELETE FROM search_index WHERE source = 'document' AND ref_id = old.id;
END;
