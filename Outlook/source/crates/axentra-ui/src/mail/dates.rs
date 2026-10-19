//! Follow-up flag dates and snooze times (docs/spec/mail.md §8.1, §10.7).

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone, Timelike, Utc, Weekday};

/// Flag presets of the Flag menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagPreset {
    Today,
    Tomorrow,
    ThisWeek,
    NextWeek,
    NoDate,
}

impl FlagPreset {
    pub const ALL: [FlagPreset; 5] = [FlagPreset::Today, FlagPreset::Tomorrow, FlagPreset::ThisWeek, FlagPreset::NextWeek, FlagPreset::NoDate];

    pub fn id(self) -> &'static str {
        match self {
            FlagPreset::Today => "today",
            FlagPreset::Tomorrow => "tomorrow",
            FlagPreset::ThisWeek => "this_week",
            FlagPreset::NextWeek => "next_week",
            FlagPreset::NoDate => "no_date",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FlagPreset::Today => "Today",
            FlagPreset::Tomorrow => "Tomorrow",
            FlagPreset::ThisWeek => "This week",
            FlagPreset::NextWeek => "Next week",
            FlagPreset::NoDate => "No date",
        }
    }

    pub fn parse(id: &str) -> Option<FlagPreset> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }
}

fn week_start_of(d: NaiveDate, week_start: Weekday) -> NaiveDate {
    d - Duration::days(((d.weekday().num_days_from_monday() + 7 - week_start.num_days_from_monday()) % 7) as i64)
}

fn workdays_in_week(start: NaiveDate, work_days: &[Weekday]) -> Vec<NaiveDate> {
    let days: Vec<NaiveDate> = (0..7).map(|i| start + Duration::days(i)).collect();
    let work: Vec<NaiveDate> = days.iter().copied().filter(|d| work_days.contains(&d.weekday())).collect();
    if work.is_empty() { days } else { work }
}

/// Due date for a flag preset (`None` for "No date"). "This week" ends on
/// the week's last workday; "Next week" on next week's last workday.
pub fn flag_due(preset: FlagPreset, today: NaiveDate, week_start: Weekday, work_days: &[Weekday]) -> Option<NaiveDate> {
    let week = week_start_of(today, week_start);
    match preset {
        FlagPreset::Today => Some(today),
        FlagPreset::Tomorrow => Some(today + Duration::days(1)),
        FlagPreset::ThisWeek => {
            let last = *workdays_in_week(week, work_days).last()?;
            Some(last.max(today))
        }
        FlagPreset::NextWeek => workdays_in_week(week + Duration::days(7), work_days).last().copied(),
        FlagPreset::NoDate => None,
    }
}

/// Snooze presets of the Snooze menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnoozePreset {
    LaterToday,
    Tomorrow,
    ThisWeekend,
    NextWeek,
}

impl SnoozePreset {
    pub const ALL: [SnoozePreset; 4] = [SnoozePreset::LaterToday, SnoozePreset::Tomorrow, SnoozePreset::ThisWeekend, SnoozePreset::NextWeek];

    pub fn id(self) -> &'static str {
        match self {
            SnoozePreset::LaterToday => "later_today",
            SnoozePreset::Tomorrow => "tomorrow",
            SnoozePreset::ThisWeekend => "this_weekend",
            SnoozePreset::NextWeek => "next_week",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SnoozePreset::LaterToday => "Later today",
            SnoozePreset::Tomorrow => "Tomorrow",
            SnoozePreset::ThisWeekend => "This weekend",
            SnoozePreset::NextWeek => "Next week",
        }
    }

    pub fn parse(id: &str) -> Option<SnoozePreset> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }
}

fn at_local(d: NaiveDate, h: u32, m: u32) -> DateTime<Utc> {
    let naive = d.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap_or(NaiveTime::MIN));
    Local.from_local_datetime(&naive).earliest().map(|t| t.with_timezone(&Utc)).unwrap_or_else(|| naive.and_utc())
}

/// When a snooze preset brings the message back, or `None` when the preset
/// isn't offered right now ("Later today" after 6 PM, "This weekend" on
/// the weekend).
pub fn snooze_until(preset: SnoozePreset, now: DateTime<Local>) -> Option<DateTime<Utc>> {
    let today = now.date_naive();
    match preset {
        SnoozePreset::LaterToday => {
            if now.hour() >= 18 {
                return None;
            }
            let t = now + Duration::hours(3);
            let (h, m) = (t.hour(), t.minute());
            let rounded = if m == 0 && t.second() == 0 { (h, 0) } else if m <= 30 { (h, 30) } else { (h + 1, 0) };
            let day = t.date_naive();
            Some(if rounded.0 >= 24 { at_local(day + Duration::days(1), 0, 0) } else { at_local(day, rounded.0, rounded.1) })
        }
        SnoozePreset::Tomorrow => Some(at_local(today + Duration::days(1), 8, 0)),
        SnoozePreset::ThisWeekend => {
            if matches!(today.weekday(), Weekday::Sat | Weekday::Sun) {
                return None;
            }
            let days = (Weekday::Sat.num_days_from_monday() - today.weekday().num_days_from_monday()) as i64;
            Some(at_local(today + Duration::days(days), 8, 0))
        }
        SnoozePreset::NextWeek => {
            let days = 7 - today.weekday().num_days_from_monday() as i64;
            Some(at_local(today + Duration::days(days), 8, 0))
        }
    }
}

/// "Tue 8:00 AM" style label for menus and toasts.
pub fn short_when(t: DateTime<Utc>, now: DateTime<Local>, h24: bool) -> String {
    let l = t.with_timezone(&Local);
    let time = crate::fmt::time(t, h24);
    if l.date_naive() == now.date_naive() { time } else { format!("{} {time}", l.format("%a")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: [Weekday; 5] = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri];

    #[test]
    fn flag_presets_use_the_work_week() {
        let d = |m, dd| NaiveDate::from_ymd_opt(2026, m, dd).unwrap();
        let tue = d(10, 20);
        assert_eq!(flag_due(FlagPreset::Today, tue, Weekday::Sun, &WORK), Some(tue));
        assert_eq!(flag_due(FlagPreset::Tomorrow, tue, Weekday::Sun, &WORK), Some(d(10, 21)));
        assert_eq!(flag_due(FlagPreset::ThisWeek, tue, Weekday::Sun, &WORK), Some(d(10, 23)));
        assert_eq!(flag_due(FlagPreset::NextWeek, tue, Weekday::Sun, &WORK), Some(d(10, 30)));
        assert_eq!(flag_due(FlagPreset::NoDate, tue, Weekday::Sun, &WORK), None);
        // Saturday: the week's last workday has passed, so it's due today.
        assert_eq!(flag_due(FlagPreset::ThisWeek, d(10, 24), Weekday::Sun, &WORK), Some(d(10, 24)));
        assert_eq!(FlagPreset::parse("next_week"), Some(FlagPreset::NextWeek));
    }

    #[test]
    fn snooze_presets() {
        let at = |dd, h, m| Local.with_ymd_and_hms(2026, 10, dd, h, m, 0).unwrap();
        let utc = |dd, h, m| at(dd, h, m).with_timezone(&Utc);
        // Monday 10:10 -> 13:30 later today.
        assert_eq!(snooze_until(SnoozePreset::LaterToday, at(19, 10, 10)), Some(utc(19, 13, 30)));
        assert_eq!(snooze_until(SnoozePreset::LaterToday, at(19, 10, 40)), Some(utc(19, 14, 0)));
        assert_eq!(snooze_until(SnoozePreset::LaterToday, at(19, 19, 0)), None);
        assert_eq!(snooze_until(SnoozePreset::Tomorrow, at(19, 22, 0)), Some(utc(20, 8, 0)));
        assert_eq!(snooze_until(SnoozePreset::ThisWeekend, at(19, 9, 0)), Some(utc(24, 8, 0)));
        assert_eq!(snooze_until(SnoozePreset::ThisWeekend, at(24, 9, 0)), None);
        assert_eq!(snooze_until(SnoozePreset::NextWeek, at(21, 9, 0)), Some(utc(26, 8, 0)));
        assert_eq!(snooze_until(SnoozePreset::NextWeek, at(25, 9, 0)), Some(utc(26, 8, 0)));
    }
}
