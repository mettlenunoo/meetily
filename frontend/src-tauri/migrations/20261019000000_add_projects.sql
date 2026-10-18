-- Projects: the thread that ties recordings (and later Slack threads and documents) together.
-- A course is a project with kind = 'course'; the UI shows courses in a separate Study area.

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL COLLATE NOCASE UNIQUE,
    kind TEXT NOT NULL DEFAULT 'work' CHECK (kind IN ('work', 'client', 'personal', 'course')),
    description TEXT,
    people TEXT,            -- comma-separated names that point to this project
    keywords TEXT,          -- comma-separated words that point to this project
    slack_channels TEXT,    -- comma-separated channel names, for the Slack bot later
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    -- 'claude': visible to hearken-mcp; 'local': hidden from it entirely
    privacy TEXT NOT NULL DEFAULT 'claude' CHECK (privacy IN ('claude', 'local')),
    color TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- One main project per meeting, plus any number of extra ones.
CREATE TABLE IF NOT EXISTS meeting_projects (
    meeting_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    is_primary INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    PRIMARY KEY (meeting_id, project_id),
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_meeting_projects_one_primary
    ON meeting_projects(meeting_id) WHERE is_primary = 1;
CREATE INDEX IF NOT EXISTS idx_meeting_projects_project ON meeting_projects(project_id);

-- A project Hearken suggests for a meeting. Nothing is linked until the user accepts.
CREATE TABLE IF NOT EXISTS project_suggestions (
    meeting_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    reason TEXT,
    source TEXT NOT NULL CHECK (source IN ('match', 'ai')),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'rejected')),
    created_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

-- Action-item tracking: a real date next to the spoken one, and when we last reminded.
ALTER TABLE meeting_items ADD COLUMN due_at TEXT;        -- YYYY-MM-DD
ALTER TABLE meeting_items ADD COLUMN reminded_on TEXT;   -- local date of the last reminder
CREATE INDEX IF NOT EXISTS idx_meeting_items_due ON meeting_items(kind, status, due_at);
