//! Turns a due date as spoken in a meeting ("Friday", "end of month", "14 Oct")
//! into a calendar date, relative to the day of the meeting.
//!
//! Same rule as owners and quotes: never guess. Only phrases with one reasonable
//! reading are resolved; anything ambiguous returns None and the spoken text stays
//! for the user to turn into a date. Examples of what is deliberately NOT resolved:
//! "next Friday" (this coming one or the week after?), "next week" (which day?),
//! "05/10" (5 Oct or 10 May?), and a weekday said on that same weekday.

use chrono::{Datelike, Duration, NaiveDate, Weekday};

const WEEKDAYS: &[(&str, Weekday)] = &[
    ("monday", Weekday::Mon), ("mon", Weekday::Mon),
    ("tuesday", Weekday::Tue), ("tue", Weekday::Tue), ("tues", Weekday::Tue),
    ("wednesday", Weekday::Wed), ("wed", Weekday::Wed),
    ("thursday", Weekday::Thu), ("thu", Weekday::Thu), ("thur", Weekday::Thu), ("thurs", Weekday::Thu),
    ("friday", Weekday::Fri), ("fri", Weekday::Fri),
    ("saturday", Weekday::Sat), ("sat", Weekday::Sat),
    ("sunday", Weekday::Sun), ("sun", Weekday::Sun),
];

const MONTHS: &[(&str, u32)] = &[
    ("january", 1), ("jan", 1), ("february", 2), ("feb", 2), ("march", 3), ("mar", 3),
    ("april", 4), ("apr", 4), ("may", 5), ("june", 6), ("jun", 6), ("july", 7), ("jul", 7),
    ("august", 8), ("aug", 8), ("september", 9), ("sep", 9), ("sept", 9), ("october", 10), ("oct", 10),
    ("november", 11), ("nov", 11), ("december", 12), ("dec", 12),
];

const FILLER: &[&str] = &["by", "on", "before", "due", "the", "this", "until", "till", "no", "later", "than", "of", "at"];

fn weekday(word: &str) -> Option<Weekday> {
    WEEKDAYS.iter().find(|(w, _)| *w == word).map(|(_, d)| *d)
}

fn month(word: &str) -> Option<u32> {
    MONTHS.iter().find(|(m, _)| *m == word).map(|(_, n)| *n)
}

/// "14th" -> 14
fn day_number(word: &str) -> Option<u32> {
    let digits = word.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &word[digits.len()..];
    if !matches!(suffix, "" | "st" | "nd" | "rd" | "th") {
        return None;
    }
    digits.parse().ok().filter(|d| (1..=31).contains(d))
}

fn last_day_of_month(year: i32, month: u32) -> Option<NaiveDate> {
    let (y, m) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    NaiveDate::from_ymd_opt(y, m, 1)?.pred_opt()
}

/// A day and month with no year: the next time it comes round, from the meeting day on.
fn next_occurrence(meeting_day: NaiveDate, month: u32, day: u32) -> Option<NaiveDate> {
    let this_year = NaiveDate::from_ymd_opt(meeting_day.year(), month, day)?;
    if this_year >= meeting_day {
        Some(this_year)
    } else {
        NaiveDate::from_ymd_opt(meeting_day.year() + 1, month, day)
    }
}

fn number_word(word: &str) -> Option<i64> {
    match word {
        "a" | "an" | "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        _ => word.parse().ok().filter(|n: &i64| (1..=60).contains(n)),
    }
}

pub fn resolve_due(spoken: &str, meeting_day: NaiveDate) -> Option<NaiveDate> {
    let lower = spoken.trim().to_lowercase();
    if lower.is_empty() {
        return None;
    }

    // ISO date, said or typed exactly
    if let Ok(d) = NaiveDate::parse_from_str(&lower, "%Y-%m-%d") {
        return Some(d);
    }

    // Numeric day/month (Ghana and UK order). Only when it can't be read the other way round.
    if let Some((a, b)) = lower.split_once('/') {
        let (a, b) = (a.trim().parse::<u32>().ok()?, b.trim().split('/').next()?.trim().parse::<u32>().ok()?);
        if lower.matches('/').count() > 1 || a <= 12 && b <= 12 && a != b {
            return None; // "05/10" or a full date with year in unknown order
        }
        return next_occurrence(meeting_day, b, a);
    }

    let words: Vec<String> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !FILLER.contains(w))
        .map(str::to_string)
        .collect();
    let w: Vec<&str> = words.iter().map(String::as_str).collect();

    match w.as_slice() {
        ["today"] | ["tonight"] | ["eod"] | ["end", "day"] | ["close", "business"] | ["cob"] => Some(meeting_day),
        ["tomorrow"] => Some(meeting_day + Duration::days(1)),
        ["end", "week"] | ["eow"] => {
            let from_monday = meeting_day.weekday().num_days_from_monday() as i64;
            if from_monday > 4 {
                None // said on a Saturday or Sunday: this week's Friday has passed
            } else {
                Some(meeting_day + Duration::days(4 - from_monday))
            }
        }
        ["end", "month"] | ["eom"] => last_day_of_month(meeting_day.year(), meeting_day.month()),
        ["in", n, unit] | [n, unit] if number_word(n).is_some() && matches!(*unit, "day" | "days" | "week" | "weeks") => {
            let n = number_word(n)?;
            let days = if unit.starts_with("week") { n * 7 } else { n };
            Some(meeting_day + Duration::days(days))
        }
        [day] if weekday(day).is_some() => {
            let target = weekday(day)?;
            let ahead = (7 + target.num_days_from_monday() as i64
                - meeting_day.weekday().num_days_from_monday() as i64) % 7;
            if ahead == 0 {
                None // "Friday" said on a Friday: today or next week?
            } else {
                Some(meeting_day + Duration::days(ahead))
            }
        }
        // "14 October", "14th Oct", "Friday 14 October"
        [d, m] | [_, d, m] if day_number(d).is_some() && month(m).is_some() => {
            next_occurrence(meeting_day, month(m)?, day_number(d)?)
        }
        // "October 14", "Oct 14th", "Friday October 14"
        [m, d] | [_, m, d] if day_number(d).is_some() && month(m).is_some() => {
            next_occurrence(meeting_day, month(m)?, day_number(d)?)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Wednesday 14 October 2026
    fn wed() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 14).unwrap()
    }
    fn d(m: u32, day: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(2026, m, day)
    }

    #[test]
    fn weekdays_resolve_to_the_next_one() {
        assert_eq!(resolve_due("Friday", wed()), d(10, 16));
        assert_eq!(resolve_due("by Friday", wed()), d(10, 16));
        assert_eq!(resolve_due("this Friday", wed()), d(10, 16));
        assert_eq!(resolve_due("on Monday", wed()), d(10, 19));
        assert_eq!(resolve_due("tues", wed()), d(10, 20));
    }

    #[test]
    fn ambiguous_phrases_stay_unresolved() {
        assert_eq!(resolve_due("Wednesday", wed()), None, "said on a Wednesday");
        assert_eq!(resolve_due("next Friday", wed()), None);
        assert_eq!(resolve_due("next week", wed()), None);
        assert_eq!(resolve_due("soon", wed()), None);
        assert_eq!(resolve_due("05/10", wed()), None, "5 Oct or 10 May");
        assert_eq!(resolve_due("Q4", wed()), None);
        assert_eq!(resolve_due("", wed()), None);
    }

    #[test]
    fn relative_phrases() {
        assert_eq!(resolve_due("today", wed()), d(10, 14));
        assert_eq!(resolve_due("end of day", wed()), d(10, 14));
        assert_eq!(resolve_due("tomorrow", wed()), d(10, 15));
        assert_eq!(resolve_due("end of the week", wed()), d(10, 16));
        assert_eq!(resolve_due("by end of month", wed()), d(10, 31));
        assert_eq!(resolve_due("in two weeks", wed()), d(10, 28));
        assert_eq!(resolve_due("in 3 days", wed()), d(10, 17));
        let saturday = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        assert_eq!(resolve_due("end of week", saturday), None);
    }

    #[test]
    fn explicit_dates() {
        assert_eq!(resolve_due("14 Oct", wed()), d(10, 14));
        assert_eq!(resolve_due("23rd October", wed()), d(10, 23));
        assert_eq!(resolve_due("Oct 30th", wed()), d(10, 30));
        assert_eq!(resolve_due("Friday 23 October", wed()), d(10, 23));
        assert_eq!(resolve_due("2026-11-02", wed()), d(11, 2));
        assert_eq!(resolve_due("25/10", wed()), d(10, 25));
        assert_eq!(resolve_due("12 Jan", wed()), NaiveDate::from_ymd_opt(2027, 1, 12), "rolls into next year");
        assert_eq!(resolve_due("31 Feb", wed()), None);
    }
}
