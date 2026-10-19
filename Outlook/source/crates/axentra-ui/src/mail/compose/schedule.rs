//! Schedule send: preset times and the time field's parsing.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone, Weekday};

/// A schedule-send preset: label and the moment it resolves to.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub at: DateTime<Local>,
}

fn at(date: NaiveDate, hour: u32) -> DateTime<Local> {
    let naive = date.and_time(NaiveTime::from_hms_opt(hour, 0, 0).unwrap_or_default());
    Local.from_local_datetime(&naive).earliest().unwrap_or_else(|| Local.from_utc_datetime(&naive))
}

/// "Tomorrow morning" 8:00 AM, "Tomorrow afternoon" 1:00 PM and, from
/// Thursday through Sunday, "Monday morning" 8:00 AM (mail spec 6.6).
pub fn presets(now: DateTime<Local>) -> Vec<Preset> {
    let tomorrow = now.date_naive() + Duration::days(1);
    let mut out = vec![
        Preset { id: "tomorrow_morning", label: "Tomorrow morning", at: at(tomorrow, 8) },
        Preset { id: "tomorrow_afternoon", label: "Tomorrow afternoon", at: at(tomorrow, 13) },
    ];
    if matches!(now.weekday(), Weekday::Thu | Weekday::Fri | Weekday::Sat | Weekday::Sun) {
        let days = (7 - now.weekday().num_days_from_monday()) as i64;
        out.push(Preset { id: "monday_morning", label: "Monday morning", at: at(now.date_naive() + Duration::days(days), 8) });
    }
    out
}

/// Caption shown next to a preset: "Tue 8:00 AM".
pub fn preset_caption(t: DateTime<Local>, h24: bool) -> String {
    let time = if h24 { t.format("%H:%M").to_string() } else { t.format("%-I:%M %p").to_string() };
    format!("{} {time}", t.format("%a"))
}

/// Times offered by the time combo box, 30 minutes apart.
pub fn time_options(h24: bool) -> Vec<(NaiveTime, String)> {
    (0..48)
        .filter_map(|i| NaiveTime::from_hms_opt(i / 2, (i % 2) * 30, 0))
        .map(|t| (t, time_label(t, h24)))
        .collect()
}

pub fn time_label(t: NaiveTime, h24: bool) -> String {
    if h24 { t.format("%H:%M").to_string() } else { t.format("%-I:%M %p").to_string() }
}

/// Parse a typed time: "8:30 PM", "8pm", "20:30", "0830", "8".
pub fn parse_time(s: &str) -> Option<NaiveTime> {
    let s = s.trim().to_ascii_lowercase().replace('.', "");
    let (body, pm) = if let Some(b) = s.strip_suffix("pm").or_else(|| s.strip_suffix('p')) {
        (b.trim(), Some(true))
    } else if let Some(b) = s.strip_suffix("am").or_else(|| s.strip_suffix('a')) {
        (b.trim(), Some(false))
    } else {
        (s.as_str(), None)
    };
    let (h, m): (u32, u32) = if let Some((h, m)) = body.split_once(':') {
        (h.trim().parse().ok()?, m.trim().parse().ok()?)
    } else if body.len() > 2 && body.chars().all(|c| c.is_ascii_digit()) {
        let (h, m) = body.split_at(body.len() - 2);
        (h.parse().ok()?, m.parse().ok()?)
    } else {
        (body.parse().ok()?, 0)
    };
    let h = match pm {
        Some(true) if (1..12).contains(&h) => h + 12,
        Some(true) if h == 12 => 12,
        Some(false) if h == 12 => 0,
        Some(_) if !(1..=12).contains(&h) => return None,
        _ => h,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// Combine a picked date and time into a local moment.
pub fn combine(date: NaiveDate, time: NaiveTime) -> DateTime<Local> {
    let naive = date.and_time(time);
    Local.from_local_datetime(&naive).earliest().unwrap_or_else(|| Local.from_utc_datetime(&naive))
}

/// Default custom time: the next whole half hour at least 30 minutes away.
pub fn default_custom(now: DateTime<Local>) -> (NaiveDate, NaiveTime) {
    let later = now + Duration::minutes(30);
    let mins = later.minute_of_day();
    let rounded = mins.div_ceil(30) * 30;
    if rounded >= 24 * 60 {
        (later.date_naive() + Duration::days(1), NaiveTime::MIN)
    } else {
        (later.date_naive(), NaiveTime::from_hms_opt(rounded / 60, rounded % 60, 0).unwrap_or_default())
    }
}

trait MinuteOfDay {
    fn minute_of_day(&self) -> u32;
}

impl MinuteOfDay for DateTime<Local> {
    fn minute_of_day(&self) -> u32 {
        use chrono::Timelike;
        self.hour() * 60 + self.minute()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn presets_follow_the_weekday() {
        // Monday 19 Oct 2026: no "Monday morning".
        let p = presets(local(2026, 10, 19, 15, 10));
        assert_eq!(p.iter().map(|p| p.label).collect::<Vec<_>>(), vec!["Tomorrow morning", "Tomorrow afternoon"]);
        assert_eq!(p[0].at, local(2026, 10, 20, 8, 0));
        assert_eq!(p[1].at, local(2026, 10, 20, 13, 0));
        // Thursday 22 Oct: Monday morning is the 26th.
        let p = presets(local(2026, 10, 22, 9, 0));
        assert_eq!(p[2].label, "Monday morning");
        assert_eq!(p[2].at, local(2026, 10, 26, 8, 0));
        // Sunday: Monday morning is tomorrow.
        let p = presets(local(2026, 10, 25, 9, 0));
        assert_eq!(p[2].at, local(2026, 10, 26, 8, 0));
        assert_eq!(preset_caption(p[2].at, false), "Mon 8:00 AM");
    }

    #[test]
    fn parses_typed_times() {
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0);
        assert_eq!(parse_time("8:30 PM"), t(20, 30));
        assert_eq!(parse_time("8pm"), t(20, 0));
        assert_eq!(parse_time("12:00 AM"), t(0, 0));
        assert_eq!(parse_time("12 pm"), t(12, 0));
        assert_eq!(parse_time("20:45"), t(20, 45));
        assert_eq!(parse_time("0830"), t(8, 30));
        assert_eq!(parse_time("7 a.m."), t(7, 0));
        assert_eq!(parse_time("13 pm"), None);
        assert_eq!(parse_time("25:00"), None);
        assert_eq!(parse_time("soon"), None);
        assert_eq!(time_options(false).len(), 48);
        assert_eq!(time_options(false)[17].1, "8:30 AM");
    }

    #[test]
    fn default_custom_rounds_up() {
        assert_eq!(default_custom(local(2026, 10, 19, 15, 10)), (NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(), NaiveTime::from_hms_opt(16, 0, 0).unwrap()));
        assert_eq!(default_custom(local(2026, 10, 19, 23, 40)).0, NaiveDate::from_ymd_opt(2026, 10, 20).unwrap());
    }
}
