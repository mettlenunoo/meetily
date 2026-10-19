/// Embedded default templates using compile-time inclusion
///
/// These templates are bundled into the binary and serve as fallbacks
/// when custom templates are not available.

/// Daily standup template for engineering/product teams
pub const DAILY_STANDUP: &str = include_str!("../../../templates/daily_standup.json");

/// Standard meeting notes template
pub const STANDARD_MEETING: &str = include_str!("../../../templates/standard_meeting.json");

/// Study notes for a class or lecture
pub const LECTURE_NOTES: &str = include_str!("../../../templates/lecture_notes.json");

/// An interview or conversation with one main guest
pub const INTERVIEW: &str = include_str!("../../../templates/interview.json");

/// A note recorded by one person
pub const VOICE_NOTE: &str = include_str!("../../../templates/voice_note.json");

/// Formal minutes
pub const MEETING_MINUTES: &str = include_str!("../../../templates/meeting_minutes.json");

/// A formal, shareable meeting report
pub const MEETING_REPORT: &str = include_str!("../../../templates/meeting_report.json");

/// Status of a project across its meetings
pub const PROJECT_STATUS_REPORT: &str = include_str!("../../../templates/project_status_report.json");

/// Registry of all built-in templates
///
/// Maps template identifiers to their embedded JSON content
pub fn get_builtin_templates() -> Vec<(&'static str, &'static str)> {
    vec![
        ("daily_standup", DAILY_STANDUP),
        ("standard_meeting", STANDARD_MEETING),
        ("lecture_notes", LECTURE_NOTES),
        ("interview", INTERVIEW),
        ("voice_note", VOICE_NOTE),
        ("meeting_minutes", MEETING_MINUTES),
        ("meeting_report", MEETING_REPORT),
        ("project_status_report", PROJECT_STATUS_REPORT),
    ]
}

/// Get a built-in template by identifier
///
/// # Arguments
/// * `id` - Template identifier (e.g., "daily_standup", "standard_meeting")
///
/// # Returns
/// The template JSON content if found, None otherwise
pub fn get_builtin_template(id: &str) -> Option<&'static str> {
    match id {
        "daily_standup" => Some(DAILY_STANDUP),
        "standard_meeting" => Some(STANDARD_MEETING),
        "lecture_notes" => Some(LECTURE_NOTES),
        "interview" => Some(INTERVIEW),
        "voice_note" => Some(VOICE_NOTE),
        "meeting_minutes" => Some(MEETING_MINUTES),
        "meeting_report" => Some(MEETING_REPORT),
        "project_status_report" => Some(PROJECT_STATUS_REPORT),
        _ => None,
    }
}

/// List all built-in template identifiers
pub fn list_builtin_template_ids() -> Vec<&'static str> {
    vec![
        "daily_standup",
        "standard_meeting",
        "lecture_notes",
        "interview",
        "voice_note",
        "meeting_minutes",
        "meeting_report",
        "project_status_report",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_templates_valid_json() {
        for (id, content) in get_builtin_templates() {
            let result = serde_json::from_str::<serde_json::Value>(content);
            assert!(
                result.is_ok(),
                "Built-in template '{}' contains invalid JSON: {:?}",
                id,
                result.err()
            );
        }
    }

    #[test]
    fn test_get_builtin_template() {
        assert!(get_builtin_template("daily_standup").is_some());
        assert!(get_builtin_template("standard_meeting").is_some());
        assert!(get_builtin_template("nonexistent").is_none());
    }
}
