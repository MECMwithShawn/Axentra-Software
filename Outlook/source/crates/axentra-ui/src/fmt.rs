//! Date, time and size formatting in Outlook's style (en-US).

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};

/// "10:30 AM" (or "10:30" with 24-hour time).
pub fn time(t: DateTime<Utc>, h24: bool) -> String {
    let l = t.with_timezone(&Local);
    if h24 { l.format("%H:%M").to_string() } else { l.format("%-I:%M %p").to_string() }
}

/// Hour label for calendar gutters: "1 PM" / "13:00".
pub fn hour_label(hour: u32, h24: bool) -> String {
    if h24 {
        format!("{hour:02}:00")
    } else {
        let h = match hour % 12 { 0 => 12, h => h };
        format!("{h} {}", if hour < 12 { "AM" } else { "PM" })
    }
}

/// Message-list timestamp: time for today, "Sun 8:16 PM" within the last
/// week, otherwise "10/17/2026".
pub fn list_time(t: DateTime<Utc>, now: DateTime<Local>, h24: bool) -> String {
    let l = t.with_timezone(&Local);
    let today = now.date_naive();
    let d = l.date_naive();
    if d == today {
        time(t, h24)
    } else if d > today - Duration::days(7) && d < today {
        format!("{} {}", l.format("%a"), time(t, h24))
    } else {
        l.format("%-m/%-d/%Y").to_string()
    }
}

/// Reading-pane timestamp: "Sun 10/18/2026 4:52 PM".
pub fn full_time(t: DateTime<Utc>, h24: bool) -> String {
    let l = t.with_timezone(&Local);
    format!("{} {}", l.format("%a %-m/%-d/%Y"), time(t, h24))
}

/// Long date: "Monday, October 19, 2026".
pub fn long_date(d: NaiveDate) -> String {
    d.format("%A, %B %-d, %Y").to_string()
}

/// Short date: "Mon, Oct 19".
pub fn short_date(d: NaiveDate) -> String {
    d.format("%a, %b %-d").to_string()
}

/// Message-list date group heading, Outlook's buckets.
pub fn date_group(t: DateTime<Utc>, now: DateTime<Local>, week_start: chrono::Weekday) -> &'static str {
    let d = t.with_timezone(&Local).date_naive();
    let today = now.date_naive();
    let start_of_week = today - Duration::days(((today.weekday().num_days_from_monday() + 7 - week_start.num_days_from_monday()) % 7) as i64);
    let start_of_last_week = start_of_week - Duration::days(7);
    let start_of_month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let start_of_last_month = start_of_month.checked_sub_months(chrono::Months::new(1)).unwrap_or(start_of_month);
    if d > today {
        "Upcoming"
    } else if d == today {
        "Today"
    } else if d == today - Duration::days(1) {
        "Yesterday"
    } else if d >= start_of_week {
        "This week"
    } else if d >= start_of_last_week {
        "Last week"
    } else if d >= start_of_month {
        "This month"
    } else if d >= start_of_last_month {
        "Last month"
    } else {
        "Older"
    }
}

/// "2.4 MB", "812 KB", "96 bytes".
pub fn file_size(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    if b < 1024.0 {
        format!("{} bytes", bytes.max(0))
    } else if b < 1024.0 * 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        let mb = b / (1024.0 * 1024.0);
        if mb < 10.0 { format!("{mb:.1} MB") } else { format!("{mb:.0} MB") }
    } else {
        format!("{:.1} GB", b / (1024.0 * 1024.0 * 1024.0))
    }
}

/// Duration label used by event forms: "30 minutes", "1 hour", "1.5 hours", "2 days".
pub fn duration_label(d: Duration) -> String {
    let mins = d.num_minutes();
    if mins < 60 {
        format!("{mins} minute{}", if mins == 1 { "" } else { "s" })
    } else if mins % (60 * 24) == 0 {
        let days = mins / (60 * 24);
        format!("{days} day{}", if days == 1 { "" } else { "s" })
    } else if mins % 60 == 0 {
        let h = mins / 60;
        format!("{h} hour{}", if h == 1 { "" } else { "s" })
    } else {
        format!("{} hours", (mins as f64 / 60.0 * 10.0).round() / 10.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn list_time_buckets() {
        let now = Local.with_ymd_and_hms(2026, 10, 19, 11, 30, 0).unwrap();
        let t = |d: u32, h: u32| Local.with_ymd_and_hms(2026, 10, d, h, 16, 0).unwrap().with_timezone(&Utc);
        assert_eq!(list_time(t(19, 10), now, false), "10:16 AM");
        assert_eq!(list_time(t(18, 20), now, false), "Sun 8:16 PM");
        assert_eq!(list_time(t(10, 9), now, false), "10/10/2026");
        assert_eq!(list_time(t(19, 13), now, true), "13:16");
    }

    #[test]
    fn date_groups() {
        let now = Local.with_ymd_and_hms(2026, 10, 22, 9, 0, 0).unwrap(); // Thursday
        let t = |m: u32, d: u32| Local.with_ymd_and_hms(2026, m, d, 8, 0, 0).unwrap().with_timezone(&Utc);
        let ws = chrono::Weekday::Sun;
        assert_eq!(date_group(t(10, 22), now, ws), "Today");
        assert_eq!(date_group(t(10, 21), now, ws), "Yesterday");
        assert_eq!(date_group(t(10, 18), now, ws), "This week");
        assert_eq!(date_group(t(10, 12), now, ws), "Last week");
        assert_eq!(date_group(t(10, 2), now, ws), "This month");
        assert_eq!(date_group(t(9, 10), now, ws), "Last month");
        assert_eq!(date_group(t(6, 1), now, ws), "Older");
    }

    #[test]
    fn sizes_and_durations() {
        assert_eq!(file_size(96), "96 bytes");
        assert_eq!(file_size(412_000), "402 KB");
        assert_eq!(file_size(2_480_000), "2.4 MB");
        assert_eq!(file_size(18_200_000), "17 MB");
        assert_eq!(duration_label(Duration::minutes(30)), "30 minutes");
        assert_eq!(duration_label(Duration::minutes(90)), "1.5 hours");
        assert_eq!(duration_label(Duration::hours(1)), "1 hour");
        assert_eq!(duration_label(Duration::days(2)), "2 days");
        assert_eq!(hour_label(0, false), "12 AM");
        assert_eq!(hour_label(13, false), "1 PM");
    }
}
