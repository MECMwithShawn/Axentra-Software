//! Due date, reminder and repeat rules and labels (spec 3.10).

use axentra_core::*;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, Timelike, Utc, Weekday};

/// Today's local date.
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// "Next week": the next Monday after `today`.
pub fn next_week(today: NaiveDate) -> NaiveDate {
    let ahead = 7 - today.weekday().num_days_from_monday() as i64;
    today + Duration::days(ahead)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DueState {
    Overdue,
    Today,
    Future,
}

pub fn due_state(due: NaiveDate, today: NaiveDate) -> DueState {
    if due < today {
        DueState::Overdue
    } else if due == today {
        DueState::Today
    } else {
        DueState::Future
    }
}

/// Due label in a task row: "Due Today", "Due Tomorrow", "Due Yesterday",
/// "Due Tue, Oct 20", "Due Tue, Oct 20, 2027".
pub fn due_row_label(due: NaiveDate, today: NaiveDate) -> String {
    match relative_day(due, today) {
        Some(rel) => format!("Due {rel}"),
        None => format!("Due {}", date_label(due, today)),
    }
}

/// Due label in the detail pane: "Today", "Tomorrow", "Yesterday", otherwise
/// the row label.
pub fn due_pane_label(due: NaiveDate, today: NaiveDate) -> String {
    relative_day(due, today).map(str::to_string).unwrap_or_else(|| due_row_label(due, today))
}

fn relative_day(d: NaiveDate, today: NaiveDate) -> Option<&'static str> {
    match (d - today).num_days() {
        0 => Some("Today"),
        1 => Some("Tomorrow"),
        -1 => Some("Yesterday"),
        _ => None,
    }
}

/// "Tue, Oct 20", with the year when it isn't this year.
pub fn date_label(d: NaiveDate, today: NaiveDate) -> String {
    if d.year() == today.year() { d.format("%a, %b %-d").to_string() } else { d.format("%a, %b %-d, %Y").to_string() }
}

/// Short weekday shown at the right of quick-date menu items, e.g. "Tue".
pub fn weekday_short(d: NaiveDate) -> String {
    d.format("%a").to_string()
}

fn local_at(d: NaiveDate, h: u32, m: u32) -> DateTime<Utc> {
    local_to_utc(d, NaiveTime::from_hms_opt(h, m, 0).unwrap_or_default())
}

/// "Later today": now + 3 hours rounded up to the hour; `None` after 21:00.
pub fn reminder_later_today(now: DateTime<Local>) -> Option<DateTime<Utc>> {
    if now.hour() >= 21 {
        return None;
    }
    let t = now + Duration::hours(3);
    let rounded = if t.minute() == 0 && t.second() == 0 { t.hour() } else { t.hour() + 1 };
    if t.date_naive() != now.date_naive() || rounded >= 24 {
        return None;
    }
    Some(local_at(now.date_naive(), rounded, 0))
}

/// "Tomorrow": 09:00 tomorrow.
pub fn reminder_tomorrow(today: NaiveDate) -> DateTime<Utc> {
    local_at(today + Duration::days(1), 9, 0)
}

/// "Next week": 09:00 next Monday.
pub fn reminder_next_week(today: NaiveDate) -> DateTime<Utc> {
    local_at(next_week(today), 9, 0)
}

/// Reminder shown in the detail pane: ("Remind me at 3:00 PM", "Today").
pub fn reminder_lines(r: DateTime<Utc>, today: NaiveDate, h24: bool) -> (String, String) {
    let d = local_date(r);
    let day = match (d - today).num_days() {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        _ => date_label(d, today),
    };
    (format!("Remind me at {}", crate::fmt::time(r, h24)), day)
}

/// Tooltip of the reminder icon in a row: "Reminder: Tue, Oct 20 9:00 AM".
pub fn reminder_tooltip(r: DateTime<Utc>, today: NaiveDate, h24: bool) -> String {
    let (_, day) = reminder_lines(r, today, h24);
    format!("Reminder: {day} {}", crate::fmt::time(r, h24))
}

/// Read a typed time: "9:30", "09:30", "930", "1730", "9 am", "9:30 PM",
/// "9.30pm". `None` when it isn't a time of day.
pub fn parse_time(s: &str) -> Option<NaiveTime> {
    let t = s.trim().to_ascii_lowercase().replace('.', ":");
    let (body, pm) = if let Some(b) = t.strip_suffix("pm").or_else(|| t.strip_suffix("p")) {
        (b.trim(), Some(true))
    } else if let Some(b) = t.strip_suffix("am").or_else(|| t.strip_suffix("a")) {
        (b.trim(), Some(false))
    } else {
        (t.as_str(), None)
    };
    let (h, m): (u32, u32) = match body.split_once(':') {
        Some((h, m)) => (h.trim().parse().ok()?, m.trim().parse().ok()?),
        None if body.len() <= 2 => (body.parse().ok()?, 0),
        None if body.len() <= 4 && body.chars().all(|c| c.is_ascii_digit()) => {
            let (h, m) = body.split_at(body.len() - 2);
            (h.parse().ok()?, m.parse().ok()?)
        }
        None => return None,
    };
    let h = match pm {
        Some(_) if !(1..=12).contains(&h) => return None,
        Some(true) => h % 12 + 12,
        Some(false) => h % 12,
        None => h,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// A time of day as the user reads it: "13:30" or "1:30 PM".
pub fn time_label(t: NaiveTime, h24: bool) -> String {
    let (h, m) = (t.hour(), t.minute());
    if h24 {
        format!("{h:02}:{m:02}")
    } else {
        let h12 = match h % 12 {
            0 => 12,
            x => x,
        };
        format!("{h12}:{m:02} {}", if h < 12 { "AM" } else { "PM" })
    }
}

/// Times offered by the reminder time picker: every 30 minutes, as
/// ("HH:MM", label).
pub fn time_options(h24: bool) -> Vec<(String, String)> {
    (0..48)
        .filter_map(|i| NaiveTime::from_hms_opt(i / 2, (i % 2) * 30, 0))
        .map(|t| (t.format("%H:%M").to_string(), time_label(t, h24)))
        .collect()
}

/// Footer text: "Created today", "Created yesterday", "Created on Mon,
/// October 12"; completed tasks say "Completed …" instead.
pub fn footer_label(t: &Task, today: NaiveDate) -> String {
    let (verb, at) = match (t.is_completed, t.completed_at) {
        (true, Some(c)) => ("Completed", c),
        _ => ("Created", t.created_at),
    };
    let d = local_date(at);
    match (today - d).num_days() {
        0 => format!("{verb} today"),
        1 => format!("{verb} yesterday"),
        _ if d.year() == today.year() => format!("{verb} on {}", d.format("%a, %B %-d")),
        _ => format!("{verb} on {}", d.format("%a, %B %-d, %Y")),
    }
}

/// "Updated a few moments ago", "Updated 5 minutes ago", "Updated 2 hours
/// ago", "Updated yesterday", "Updated on Oct 12".
pub fn updated_label(t: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let mins = (now - t).num_minutes();
    if mins < 1 {
        "Updated a few moments ago".into()
    } else if mins < 60 {
        format!("Updated {mins} minute{} ago", if mins == 1 { "" } else { "s" })
    } else if mins < 60 * 24 && local_date(t) == local_date(now) {
        let h = mins / 60;
        format!("Updated {h} hour{} ago", if h == 1 { "" } else { "s" })
    } else if local_date(t) == local_date(now) - Duration::days(1) {
        "Updated yesterday".into()
    } else {
        format!("Updated on {}", local_date(t).format("%b %-d"))
    }
}

// ---------------------------------------------------------------- repeat

const WEEKDAYS: [Weekday; 5] = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri];

/// Repeat menu presets by id ("daily", "weekdays", "weekly", "monthly", "yearly").
pub fn repeat_preset(id: &str) -> Option<Recurrence> {
    let r = |freq| Recurrence { freq, interval: 1, ..Default::default() };
    Some(match id {
        "daily" => r(RecurrenceFreq::Daily),
        "weekdays" => Recurrence { by_weekday: WEEKDAYS.to_vec(), ..r(RecurrenceFreq::Weekly) },
        "weekly" => r(RecurrenceFreq::Weekly),
        "monthly" => r(RecurrenceFreq::Monthly),
        "yearly" => r(RecurrenceFreq::Yearly),
        _ => return None,
    })
}

fn is_weekdays(r: &Recurrence) -> bool {
    r.freq == RecurrenceFreq::Weekly && r.interval <= 1 && r.by_weekday.len() == 5 && WEEKDAYS.iter().all(|d| r.by_weekday.contains(d))
}

fn day_names(days: &[Weekday]) -> String {
    let mut days = days.to_vec();
    days.sort_by_key(|d| d.num_days_from_monday());
    days.iter().map(|d| format!("{d}")).collect::<Vec<_>>().join(", ")
}

/// Short label: "Daily", "Weekdays", "Weekly", "Monthly", "Yearly",
/// "Every 2 days", "Weekly on Mon, Wed", "Every 2 weeks on Tue".
pub fn repeat_label(r: &Recurrence) -> String {
    let n = r.interval.max(1);
    match r.freq {
        RecurrenceFreq::Daily if n == 1 => "Daily".into(),
        RecurrenceFreq::Daily => format!("Every {n} days"),
        RecurrenceFreq::Weekly if is_weekdays(r) => "Weekdays".into(),
        RecurrenceFreq::Weekly if r.by_weekday.len() > 1 && n == 1 => format!("Weekly on {}", day_names(&r.by_weekday)),
        RecurrenceFreq::Weekly if r.by_weekday.len() > 1 => format!("Every {n} weeks on {}", day_names(&r.by_weekday)),
        RecurrenceFreq::Weekly if n == 1 => "Weekly".into(),
        RecurrenceFreq::Weekly => format!("Every {n} weeks"),
        RecurrenceFreq::Monthly if n == 1 => "Monthly".into(),
        RecurrenceFreq::Monthly => format!("Every {n} months"),
        RecurrenceFreq::Yearly if n == 1 => "Yearly".into(),
        RecurrenceFreq::Yearly => format!("Every {n} years"),
    }
}

/// Spelled-out rule for tooltips: "Weekly on Mon, Wed", "Monthly on day 19",
/// "Yearly on October 19". `base` is the due date the rule repeats from.
pub fn repeat_description(r: &Recurrence, base: NaiveDate) -> String {
    let n = r.interval.max(1);
    match r.freq {
        RecurrenceFreq::Daily => repeat_label(r),
        RecurrenceFreq::Weekly => {
            if is_weekdays(r) {
                return "Every weekday".into();
            }
            let days = if r.by_weekday.is_empty() { vec![base.weekday()] } else { r.by_weekday.clone() };
            if n == 1 { format!("Weekly on {}", day_names(&days)) } else { format!("Every {n} weeks on {}", day_names(&days)) }
        }
        RecurrenceFreq::Monthly => {
            let day = r.month_day.unwrap_or(base.day());
            if n == 1 { format!("Monthly on day {day}") } else { format!("Every {n} months on day {day}") }
        }
        RecurrenceFreq::Yearly => {
            let on = base.format("%B %-d");
            if n == 1 { format!("Yearly on {on}") } else { format!("Every {n} years on {on}") }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn due_labels() {
        let today = d(10, 19);
        assert_eq!(due_row_label(today, today), "Due Today");
        assert_eq!(due_pane_label(today, today), "Today");
        assert_eq!(due_row_label(d(10, 20), today), "Due Tomorrow");
        assert_eq!(due_row_label(d(10, 18), today), "Due Yesterday");
        assert_eq!(due_row_label(d(10, 12), today), "Due Mon, Oct 12");
        assert_eq!(due_pane_label(d(10, 22), today), "Due Thu, Oct 22");
        assert_eq!(due_row_label(NaiveDate::from_ymd_opt(2027, 10, 20).unwrap(), today), "Due Wed, Oct 20, 2027");
        assert_eq!(due_state(d(10, 12), today), DueState::Overdue);
        assert_eq!(due_state(today, today), DueState::Today);
    }

    #[test]
    fn quick_dates() {
        assert_eq!(next_week(d(10, 19)), d(10, 26), "Monday -> next Monday");
        assert_eq!(next_week(d(10, 25)), d(10, 26), "Sunday -> tomorrow");
        let at = |h, m| Local.with_ymd_and_hms(2026, 10, 19, h, m, 0).unwrap();
        let later = reminder_later_today(at(10, 20)).unwrap().with_timezone(&Local);
        assert_eq!((later.hour(), later.minute()), (14, 0));
        let exact = reminder_later_today(at(10, 0)).unwrap().with_timezone(&Local);
        assert_eq!(exact.hour(), 13);
        assert!(reminder_later_today(at(21, 5)).is_none());
        assert_eq!(reminder_tomorrow(d(10, 19)).with_timezone(&Local).hour(), 9);
        assert_eq!(local_date(reminder_next_week(d(10, 19))), d(10, 26));
        assert_eq!(time_options(false)[27], ("13:30".to_string(), "1:30 PM".to_string()));
        assert_eq!(time_label(NaiveTime::from_hms_opt(8, 5, 0).unwrap(), true), "08:05");
        assert_eq!(time_label(NaiveTime::from_hms_opt(0, 30, 0).unwrap(), false), "12:30 AM");
    }

    #[test]
    fn typed_times() {
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0);
        assert_eq!(parse_time("9:30"), t(9, 30));
        assert_eq!(parse_time(" 09:30 "), t(9, 30));
        assert_eq!(parse_time("1730"), t(17, 30));
        assert_eq!(parse_time("930"), t(9, 30));
        assert_eq!(parse_time("9 am"), t(9, 0));
        assert_eq!(parse_time("12 AM"), t(0, 0));
        assert_eq!(parse_time("12:15 pm"), t(12, 15));
        assert_eq!(parse_time("9:30 PM"), t(21, 30));
        assert_eq!(parse_time("9.30pm"), t(21, 30));
        assert_eq!(parse_time("1:30 PM"), t(13, 30), "the picker's own labels parse");
        assert_eq!(parse_time("13 pm"), None);
        assert_eq!(parse_time("25:00"), None);
        assert_eq!(parse_time("9:75"), None);
        assert_eq!(parse_time("soon"), None);
        assert_eq!(parse_time(""), None);
    }

    #[test]
    fn repeat_labels() {
        let r = |id| repeat_preset(id).unwrap();
        assert_eq!(repeat_label(&r("daily")), "Daily");
        assert_eq!(repeat_label(&r("weekdays")), "Weekdays");
        assert_eq!(repeat_label(&r("weekly")), "Weekly");
        assert_eq!(repeat_label(&r("monthly")), "Monthly");
        assert_eq!(repeat_label(&r("yearly")), "Yearly");
        let custom = Recurrence { interval: 2, by_weekday: vec![Weekday::Wed, Weekday::Mon], ..r("weekly") };
        assert_eq!(repeat_label(&custom), "Every 2 weeks on Mon, Wed");
        assert_eq!(repeat_description(&r("weekly"), d(10, 20)), "Weekly on Tue");
        assert_eq!(repeat_description(&r("monthly"), d(10, 20)), "Monthly on day 20");
        assert_eq!(repeat_description(&r("yearly"), d(10, 20)), "Yearly on October 20");
        assert_eq!(repeat_label(&Recurrence { interval: 3, ..r("daily") }), "Every 3 days");
    }

    #[test]
    fn footer_and_updated_labels() {
        let today = local_date(Utc::now());
        let mut t = Task { created_at: Utc::now(), ..Default::default() };
        assert_eq!(footer_label(&t, today), "Created today");
        t.created_at = Utc::now() - Duration::days(1);
        assert_eq!(footer_label(&t, today), "Created yesterday");
        t.is_completed = true;
        t.completed_at = Some(Utc::now());
        assert_eq!(footer_label(&t, today), "Completed today");
        let now = Utc::now();
        assert_eq!(updated_label(now, now), "Updated a few moments ago");
        assert_eq!(updated_label(now - Duration::minutes(5), now), "Updated 5 minutes ago");
    }
}
