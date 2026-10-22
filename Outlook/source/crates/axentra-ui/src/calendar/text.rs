//! Calendar strings: time ranges, "when" lines, labels for Show as and
//! reminders, and the forgiving date/time parsers behind the pickers.

use axentra_core::{Occurrence, ResponseStatus, ShowAs, local_date};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, Timelike, Utc};

use crate::fmt;

/// "9:30 AM" / "09:30" for a wall-clock time.
pub fn clock(t: NaiveTime, h24: bool) -> String {
    if h24 { t.format("%H:%M").to_string() } else { t.format("%-I:%M %p").to_string() }
}

/// "9:00 AM – 9:30 AM".
pub fn time_range(start: DateTime<Utc>, end: DateTime<Utc>, h24: bool) -> String {
    format!("{} – {}", fmt::time(start, h24), fmt::time(end, h24))
}

/// Short date used in pickers and peek lines: "10/19/2026".
pub fn short_date(d: NaiveDate) -> String {
    d.format("%-m/%-d/%Y").to_string()
}

/// The peek card's "when" line:
/// "Mon 10/19/2026 9:00 AM – 9:30 AM", "Mon 10/19/2026 (All day)",
/// "Mon 10/19/2026 – Wed 10/21/2026", or with times on both ends when a
/// timed event spans days. Each end is kept on one line (no-break spaces),
/// so a long line wraps after the dash, never inside "2:00 AM".
pub fn when_text(occ: &Occurrence, h24: bool) -> String {
    let keep = |s: String| s.replace(' ', "\u{a0}");
    let day = |d: NaiveDate| d.format("%a %-m/%-d/%Y").to_string();
    let range = |a: String, b: String| format!("{}\u{a0}– {}", keep(a), keep(b));
    let sd = local_date(occ.start);
    if occ.event.all_day {
        let last = local_date(occ.end) - Duration::days(1);
        if last <= sd { keep(format!("{} (All day)", day(sd))) } else { range(day(sd), day(last)) }
    } else {
        let ed = local_date(occ.end - Duration::seconds(1)).max(sd);
        if ed == sd {
            range(format!("{} {}", day(sd), fmt::time(occ.start, h24)), fmt::time(occ.end, h24))
        } else {
            range(format!("{} {}", day(sd), fmt::time(occ.start, h24)), format!("{} {}", day(local_date(occ.end)), fmt::time(occ.end, h24)))
        }
    }
}

/// Duration suffix in the end-time list: "0 min", "30 min", "1 hr", "1.5 hr".
pub fn duration_short(d: Duration) -> String {
    let m = d.num_minutes().max(0);
    if m < 60 {
        format!("{m} min")
    } else if m % 60 == 0 {
        format!("{} hr", m / 60)
    } else {
        let h = (m as f64 / 60.0 * 100.0).round() / 100.0;
        format!("{h} hr")
    }
}

/// Free-typed time: "9", "930", "9:30", "9:30p", "9.30 pm", "21:30", "12a".
pub fn parse_time(input: &str) -> Option<NaiveTime> {
    let s = input.trim().to_ascii_lowercase().replace(['.', ' '], "");
    if s.is_empty() {
        return None;
    }
    let (body, meridiem) = if let Some(b) = s.strip_suffix("am").or_else(|| s.strip_suffix('a')) {
        (b.to_string(), Some(false))
    } else if let Some(b) = s.strip_suffix("pm").or_else(|| s.strip_suffix('p')) {
        (b.to_string(), Some(true))
    } else {
        (s, None)
    };
    let (h, m): (u32, u32) = if let Some((h, m)) = body.split_once(':') {
        (h.parse().ok()?, if m.is_empty() { 0 } else { m.parse().ok()? })
    } else {
        if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        match body.len() {
            1 | 2 => (body.parse().ok()?, 0),
            3 => (body[..1].parse().ok()?, body[1..].parse().ok()?),
            4 => (body[..2].parse().ok()?, body[2..].parse().ok()?),
            _ => return None,
        }
    };
    if m > 59 {
        return None;
    }
    let h = match meridiem {
        Some(pm) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            (h % 12) + if pm { 12 } else { 0 }
        }
        None => h,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// Free-typed date in en-US order: "10/19/2026", "10/19/26", "10/19" (this
/// year), or ISO "2026-10-19".
pub fn parse_date(input: &str, today: NaiveDate) -> Option<NaiveDate> {
    let s = input.trim();
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    let parts: Vec<&str> = s.split(['/', '-', '.']).map(str::trim).collect();
    let num = |p: &str| p.parse::<i32>().ok();
    match parts.as_slice() {
        [m, d] => NaiveDate::from_ymd_opt(today.year(), num(m)? as u32, num(d)? as u32),
        [m, d, y] => {
            let y = num(y)?;
            let y = if y < 100 { 2000 + y } else { y };
            NaiveDate::from_ymd_opt(y, num(m)? as u32, num(d)? as u32)
        }
        _ => None,
    }
}

/// Reminder menu values in minutes (`None` = "Don't remind me").
pub const REMINDER_CHOICES: [Option<i32>; 10] =
    [None, Some(0), Some(5), Some(15), Some(30), Some(60), Some(120), Some(720), Some(1440), Some(10080)];

/// "Don't remind me", "At time of event", "15 minutes before", "1 hour before",
/// "18 hours before", "1 day before", "1 week before".
pub fn reminder_label(minutes: Option<i32>) -> String {
    let Some(m) = minutes else { return "Don't remind me".into() };
    let plural = |n: i32, unit: &str| format!("{n} {unit}{} before", if n == 1 { "" } else { "s" });
    match m {
        i32::MIN..=0 => "At time of event".into(),
        m if m % 10080 == 0 => plural(m / 10080, "week"),
        m if m % 1440 == 0 => plural(m / 1440, "day"),
        m if m % 60 == 0 => plural(m / 60, "hour"),
        m => plural(m, "minute"),
    }
}

/// Menu label for Show as ("Away" in menus).
pub fn show_as_label(s: ShowAs) -> &'static str {
    match s {
        ShowAs::Free => "Free",
        ShowAs::WorkingElsewhere => "Working elsewhere",
        ShowAs::Tentative => "Tentative",
        ShowAs::Busy => "Busy",
        ShowAs::OutOfOffice => "Away",
    }
}

/// Legend / tooltip label for Show as ("Out of office" there).
pub fn show_as_legend(s: ShowAs) -> &'static str {
    match s {
        ShowAs::OutOfOffice => "Out of office",
        other => show_as_label(other),
    }
}

pub const SHOW_AS_CHOICES: [ShowAs; 5] = [ShowAs::Free, ShowAs::WorkingElsewhere, ShowAs::Tentative, ShowAs::Busy, ShowAs::OutOfOffice];

pub fn show_as_id(s: ShowAs) -> &'static str {
    match s {
        ShowAs::Free => "free",
        ShowAs::WorkingElsewhere => "elsewhere",
        ShowAs::Tentative => "tentative",
        ShowAs::Busy => "busy",
        ShowAs::OutOfOffice => "oof",
    }
}

pub fn show_as_from_id(id: &str) -> Option<ShowAs> {
    SHOW_AS_CHOICES.into_iter().find(|s| show_as_id(*s) == id)
}

/// Response wording for the peek card and accessible names.
pub fn response_label(r: ResponseStatus) -> &'static str {
    match r {
        ResponseStatus::Accepted => "Accepted",
        ResponseStatus::Tentative => "Tentatively accepted",
        ResponseStatus::Declined => "Declined",
        ResponseStatus::NotResponded => "Not yet responded",
        ResponseStatus::Organizer => "Organizer",
        ResponseStatus::None => "",
    }
}

/// Local wall-clock minutes since midnight of a UTC instant.
pub fn minutes_of_day(t: DateTime<Utc>) -> i64 {
    let l = t.with_timezone(&Local);
    (l.hour() * 60 + l.minute()) as i64
}

/// The display time zone as "(UTC-06:00) America/Denver".
pub fn time_zone_label() -> String {
    let offset = Local::now().offset().local_minus_utc();
    let sign = if offset < 0 { '-' } else { '+' };
    let a = offset.abs();
    let name = std::env::var("TZ")
        .ok()
        .filter(|z| !z.is_empty())
        .or_else(|| {
            std::fs::read_link("/etc/localtime")
                .ok()
                .and_then(|p| p.to_str().and_then(|s| s.split("zoneinfo/").nth(1).map(str::to_string)))
        })
        .unwrap_or_else(|| "Local time".into());
    format!("(UTC{sign}{:02}:{:02}) {name}", a / 3600, (a % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn parses_typed_times() {
        assert_eq!(parse_time("9"), Some(t(9, 0)));
        assert_eq!(parse_time("930"), Some(t(9, 30)));
        assert_eq!(parse_time("9:30p"), Some(t(21, 30)));
        assert_eq!(parse_time("9:30 PM"), Some(t(21, 30)));
        assert_eq!(parse_time("21:30"), Some(t(21, 30)));
        assert_eq!(parse_time("1215"), Some(t(12, 15)));
        assert_eq!(parse_time("12a"), Some(t(0, 0)));
        assert_eq!(parse_time("12 pm"), Some(t(12, 0)));
        assert_eq!(parse_time("25"), None);
        assert_eq!(parse_time("13pm"), None);
        assert_eq!(parse_time("9:75"), None);
        assert_eq!(parse_time("soon"), None);
    }

    #[test]
    fn parses_typed_dates() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day);
        assert_eq!(parse_date("10/19/2026", today), d(10, 19));
        assert_eq!(parse_date("11/2/26", today), d(11, 2));
        assert_eq!(parse_date("12/31", today), d(12, 31));
        assert_eq!(parse_date("2026-10-21", today), d(10, 21));
        assert_eq!(parse_date("2/30/2026", today), None);
        assert_eq!(parse_date("tomorrow", today), None);
    }

    #[test]
    fn labels() {
        assert_eq!(reminder_label(None), "Don't remind me");
        assert_eq!(reminder_label(Some(0)), "At time of event");
        assert_eq!(reminder_label(Some(15)), "15 minutes before");
        assert_eq!(reminder_label(Some(60)), "1 hour before");
        assert_eq!(reminder_label(Some(1080)), "18 hours before");
        assert_eq!(reminder_label(Some(1440)), "1 day before");
        assert_eq!(reminder_label(Some(10080)), "1 week before");
        assert_eq!(duration_short(Duration::minutes(0)), "0 min");
        assert_eq!(duration_short(Duration::minutes(30)), "30 min");
        assert_eq!(duration_short(Duration::minutes(60)), "1 hr");
        assert_eq!(duration_short(Duration::minutes(90)), "1.5 hr");
        assert_eq!(duration_short(Duration::minutes(105)), "1.75 hr");
        assert_eq!(show_as_label(ShowAs::OutOfOffice), "Away");
        assert_eq!(show_as_legend(ShowAs::OutOfOffice), "Out of office");
        assert_eq!(show_as_from_id("elsewhere"), Some(ShowAs::WorkingElsewhere));
        assert_eq!(clock(t(9, 5), false), "9:05 AM");
        assert_eq!(clock(t(9, 5), true), "09:05");
    }

    #[test]
    fn when_lines() {
        use axentra_core::{Event, local_to_utc};
        let day = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let occ = |start: DateTime<Utc>, end: DateTime<Utc>, all_day: bool| Occurrence {
            event: Event { start, end, all_day, ..Default::default() },
            start,
            end,
            original_start: start,
            color: String::new(),
        };
        let plain = |s: String| s.replace('\u{a0}', " ");
        let timed = occ(local_to_utc(day, t(9, 0)), local_to_utc(day, t(9, 30)), false);
        assert_eq!(plain(when_text(&timed, false)), "Mon 10/19/2026 9:00 AM – 9:30 AM");
        let one_day = occ(local_to_utc(day, t(0, 0)), local_to_utc(day + Duration::days(1), t(0, 0)), true);
        assert_eq!(plain(when_text(&one_day, false)), "Mon 10/19/2026 (All day)");
        let three = occ(local_to_utc(day, t(0, 0)), local_to_utc(day + Duration::days(3), t(0, 0)), true);
        assert_eq!(plain(when_text(&three, false)), "Mon 10/19/2026 – Wed 10/21/2026");
        let overnight = occ(local_to_utc(day, t(21, 0)), local_to_utc(day + Duration::days(1), t(2, 0)), false);
        assert_eq!(plain(when_text(&overnight, false)), "Mon 10/19/2026 9:00 PM – Tue 10/20/2026 2:00 AM");
        // The only place the line may wrap is after the dash.
        assert_eq!(when_text(&overnight, false).matches(' ').count(), 1);
        assert!(when_text(&overnight, false).contains("2:00\u{a0}AM"));
    }
}
