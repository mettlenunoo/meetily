-- Meetings shared to Slack through Desmond's bot (only when the user chooses).
CREATE TABLE IF NOT EXISTS slack_shares (
    packet_id TEXT PRIMARY KEY,     -- meeting id + content hash: the same content is never posted twice
    meeting_id TEXT NOT NULL,
    channel TEXT NOT NULL,
    ts TEXT,
    permalink TEXT,
    tasks_proposed INTEGER NOT NULL DEFAULT 0,
    decisions_saved INTEGER NOT NULL DEFAULT 0,
    shared_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_slack_shares_meeting ON slack_shares(meeting_id, shared_at);

-- The bot's tasks and decisions per project, copied in when the user refreshes.
CREATE TABLE IF NOT EXISTS slack_items (
    key TEXT PRIMARY KEY,           -- 'task:12' or 'decision:3'
    project_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('task', 'decision')),
    title TEXT NOT NULL,
    status TEXT,                    -- tasks: todo / in_progress / blocked / done
    assignee TEXT,
    due_at TEXT,                    -- YYYY-MM-DD
    overdue INTEGER NOT NULL DEFAULT 0,
    link TEXT,
    source TEXT,                    -- decisions: 'slack' or 'meeting'
    meeting_ref TEXT,
    updated_at TEXT,
    synced_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_slack_items_project ON slack_items(project_id, kind);
