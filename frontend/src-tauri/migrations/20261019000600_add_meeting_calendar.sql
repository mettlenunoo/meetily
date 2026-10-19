-- The calendar event a recording happened during (read from the Mac's own Calendar).
CREATE TABLE IF NOT EXISTS meeting_calendar (
    meeting_id TEXT PRIMARY KEY,
    event_title TEXT NOT NULL,
    calendar_name TEXT,
    starts_at TEXT NOT NULL,
    ends_at TEXT NOT NULL,
    attendees TEXT,              -- comma-separated names as in the invitation
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);
