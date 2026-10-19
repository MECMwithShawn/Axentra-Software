//! Views, visible date ranges, navigation steps and range titles. Pure date
//! maths so it can be unit-tested.

use axentra_core::CalendarView;
use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};

/// The surface's view. `Day(n)` shows n consecutive days (1–7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewKind {
    Day(u8),
    WorkWeek,
    Week,
    Month,
}

impl ViewKind {
    pub fn from_settings(v: CalendarView, day_count: u8) -> Self {
        match v {
            CalendarView::Day => ViewKind::Day(day_count.clamp(1, 7)),
            CalendarView::WorkWeek => ViewKind::WorkWeek,
            CalendarView::Week => ViewKind::Week,
            CalendarView::Month => ViewKind::Month,
        }
    }

    pub fn to_settings(self) -> CalendarView {
        match self {
            ViewKind::Day(_) => CalendarView::Day,
            ViewKind::WorkWeek => CalendarView::WorkWeek,
            ViewKind::Week => CalendarView::Week,
            ViewKind::Month => CalendarView::Month,
        }
    }

    /// Unit in "Go to previous <unit>".
    pub fn unit(self) -> &'static str {
        match self {
            ViewKind::Day(_) => "day",
            ViewKind::WorkWeek | ViewKind::Week => "week",
            ViewKind::Month => "month",
        }
    }

    pub fn label(self) -> String {
        match self {
            ViewKind::Day(1) => "Day".into(),
            ViewKind::Day(n) => format!("{n} days"),
            ViewKind::WorkWeek => "Work week".into(),
            ViewKind::Week => "Week".into(),
            ViewKind::Month => "Month".into(),
        }
    }
}

/// First day of the week containing `d`.
pub fn week_start_of(d: NaiveDate, week_start: Weekday) -> NaiveDate {
    let back = (d.weekday().num_days_from_monday() + 7 - week_start.num_days_from_monday()) % 7;
    d - Duration::days(back as i64)
}

pub fn first_of_month(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).expect("valid date")
}

fn last_of_month(d: NaiveDate) -> NaiveDate {
    first_of_month(d) + Months::new(1) - Duration::days(1)
}

/// The days the surface shows. Month view returns every cell of the grid
/// (whole weeks, 5 or 6 rows).
pub fn visible_days(view: ViewKind, anchor: NaiveDate, week_start: Weekday, work_days: &[Weekday]) -> Vec<NaiveDate> {
    match view {
        ViewKind::Day(n) => (0..n.clamp(1, 7) as i64).map(|i| anchor + Duration::days(i)).collect(),
        ViewKind::Week => {
            let s = week_start_of(anchor, week_start);
            (0..7).map(|i| s + Duration::days(i)).collect()
        }
        ViewKind::WorkWeek => {
            let s = week_start_of(anchor, week_start);
            let days: Vec<NaiveDate> = (0..7).map(|i| s + Duration::days(i)).filter(|d| work_days.contains(&d.weekday())).collect();
            if days.is_empty() { (1..6).map(|i| s + Duration::days(i)).collect() } else { days }
        }
        ViewKind::Month => month_grid(anchor, week_start),
    }
}

/// Whole weeks covering the month of `anchor`.
pub fn month_grid(anchor: NaiveDate, week_start: Weekday) -> Vec<NaiveDate> {
    let start = week_start_of(first_of_month(anchor), week_start);
    let end = week_start_of(last_of_month(anchor), week_start) + Duration::days(6);
    let n = (end - start).num_days() + 1;
    (0..n).map(|i| start + Duration::days(i)).collect()
}

/// The mini month always shows six weeks so its height never changes.
pub fn mini_month_grid(month: NaiveDate, week_start: Weekday) -> Vec<NaiveDate> {
    let start = week_start_of(first_of_month(month), week_start);
    (0..42).map(|i| start + Duration::days(i)).collect()
}

/// The anchor after moving one period back (`dir` = -1) or forward (+1).
pub fn step(view: ViewKind, anchor: NaiveDate, dir: i32) -> NaiveDate {
    match view {
        ViewKind::Day(n) => anchor + Duration::days(n.clamp(1, 7) as i64 * dir as i64),
        ViewKind::WorkWeek | ViewKind::Week => anchor + Duration::weeks(dir as i64),
        ViewKind::Month => add_months(anchor, dir),
    }
}

/// Same day of month `n` months later (clamped to the month's length).
pub fn add_months(d: NaiveDate, n: i32) -> NaiveDate {
    let first = first_of_month(d);
    let target = if n >= 0 { first + Months::new(n as u32) } else { first - Months::new(n.unsigned_abs()) };
    let len = (last_of_month(target) - target).num_days() as u32 + 1;
    NaiveDate::from_ymd_opt(target.year(), target.month(), d.day().min(len)).expect("valid date")
}

/// Surface header title, e.g. "October 19–23, 2026" (en dash).
pub fn range_title(view: ViewKind, anchor: NaiveDate, days: &[NaiveDate]) -> String {
    let (Some(&first), Some(&last)) = (days.first(), days.last()) else { return String::new() };
    match view {
        ViewKind::Month => anchor.format("%B %Y").to_string(),
        ViewKind::Day(1) => first.format("%A, %B %-d, %Y").to_string(),
        ViewKind::Day(_) => span_title(first, last, "%B"),
        ViewKind::WorkWeek | ViewKind::Week => {
            if first.month() == last.month() && first.year() == last.year() {
                span_title(first, last, "%B")
            } else {
                span_title(first, last, "%b")
            }
        }
    }
}

fn span_title(first: NaiveDate, last: NaiveDate, month_fmt: &str) -> String {
    let m = |d: NaiveDate| d.format(month_fmt).to_string();
    if first == last {
        format!("{} {}, {}", m(first), first.day(), first.year())
    } else if first.year() != last.year() {
        format!("{} {}, {} – {} {}, {}", m(first), first.day(), first.year(), m(last), last.day(), last.year())
    } else if first.month() != last.month() {
        format!("{} {} – {} {}, {}", m(first), first.day(), m(last), last.day(), last.year())
    } else {
        format!("{} {}–{}, {}", m(first), first.day(), last.day(), last.year())
    }
}

/// Polite live-region text after navigating, e.g. "Week of October 18 to 24, 2026".
pub fn announce_text(view: ViewKind, anchor: NaiveDate, days: &[NaiveDate]) -> String {
    let (Some(&first), Some(&last)) = (days.first(), days.last()) else { return String::new() };
    match view {
        ViewKind::Day(1) => first.format("%A, %B %-d, %Y").to_string(),
        ViewKind::Month => anchor.format("%B %Y").to_string(),
        _ => {
            let to = if first.month() == last.month() { last.format("%-d").to_string() } else { last.format("%B %-d").to_string() };
            let prefix = if matches!(view, ViewKind::Day(_)) { "" } else { "Week of " };
            format!("{prefix}{} to {to}, {}", first.format("%B %-d"), last.year())
        }
    }
}

/// Accessible name of the time grid / month grid.
pub fn grid_label(view: ViewKind, anchor: NaiveDate, days: &[NaiveDate]) -> String {
    let first = days.first().copied().unwrap_or(anchor);
    match view {
        ViewKind::Day(1) => format!("Calendar, {}", first.format("%A, %B %-d, %Y")),
        ViewKind::Day(n) => format!("Calendar, {n} days from {}", first.format("%B %-d, %Y")),
        ViewKind::WorkWeek => format!("Calendar, work week of {}", first.format("%B %-d, %Y")),
        ViewKind::Week => format!("Calendar, week of {}", first.format("%B %-d, %Y")),
        ViewKind::Month => format!("Calendar, {}", anchor.format("%B %Y")),
    }
}

/// Week number with the "first day of year" rule: week 1 is the week that
/// contains January 1.
pub fn week_number(d: NaiveDate, week_start: Weekday) -> u32 {
    let ws = week_start_of(d, week_start);
    let we = ws + Duration::days(6);
    if we.year() != ws.year() {
        return 1;
    }
    let jan1 = NaiveDate::from_ymd_opt(ws.year(), 1, 1).expect("valid date");
    ((ws - week_start_of(jan1, week_start)).num_days() / 7 + 1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use Weekday::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    const WORK: [Weekday; 5] = [Mon, Tue, Wed, Thu, Fri];

    #[test]
    fn visible_ranges_per_view() {
        let today = d(2026, 10, 19);
        let ww = visible_days(ViewKind::WorkWeek, today, Sun, &WORK);
        assert_eq!((ww[0], ww[4], ww.len()), (d(2026, 10, 19), d(2026, 10, 23), 5));
        let w = visible_days(ViewKind::Week, d(2026, 10, 22), Sun, &WORK);
        assert_eq!((w[0], w[6]), (d(2026, 10, 18), d(2026, 10, 24)));
        let w = visible_days(ViewKind::Week, d(2026, 10, 22), Mon, &WORK);
        assert_eq!(w[0], d(2026, 10, 19));
        assert_eq!(visible_days(ViewKind::Day(3), today, Sun, &WORK), vec![d(2026, 10, 19), d(2026, 10, 20), d(2026, 10, 21)]);
        let odd = visible_days(ViewKind::WorkWeek, today, Sun, &[Mon, Wed, Fri]);
        assert_eq!(odd, vec![d(2026, 10, 19), d(2026, 10, 21), d(2026, 10, 23)]);
        // October 2026 starts on a Thursday and needs five Sunday-first rows.
        let m = visible_days(ViewKind::Month, today, Sun, &WORK);
        assert_eq!((m.len(), m[0], *m.last().unwrap()), (35, d(2026, 9, 27), d(2026, 10, 31)));
        // August 2026 needs six.
        assert_eq!(month_grid(d(2026, 8, 10), Sun).len(), 42);
        assert_eq!(mini_month_grid(d(2026, 10, 1), Sun).len(), 42);
    }

    #[test]
    fn steps_by_period() {
        let a = d(2026, 10, 19);
        assert_eq!(step(ViewKind::Day(1), a, 1), d(2026, 10, 20));
        assert_eq!(step(ViewKind::Day(3), a, -1), d(2026, 10, 16));
        assert_eq!(step(ViewKind::Week, a, 1), d(2026, 10, 26));
        assert_eq!(step(ViewKind::Month, d(2026, 1, 31), 1), d(2026, 2, 28));
        assert_eq!(step(ViewKind::Month, d(2026, 1, 15), -1), d(2025, 12, 15));
    }

    #[test]
    fn titles_match_outlook_formats() {
        let t = |v, a: NaiveDate| range_title(v, a, &visible_days(v, a, Sun, &WORK));
        assert_eq!(t(ViewKind::Day(1), d(2026, 10, 19)), "Monday, October 19, 2026");
        assert_eq!(t(ViewKind::Day(3), d(2026, 10, 19)), "October 19–21, 2026");
        assert_eq!(t(ViewKind::Day(3), d(2026, 10, 30)), "October 30 – November 1, 2026");
        assert_eq!(t(ViewKind::Day(3), d(2026, 12, 31)), "December 31, 2026 – January 2, 2027");
        assert_eq!(t(ViewKind::WorkWeek, d(2026, 10, 19)), "October 19–23, 2026");
        assert_eq!(t(ViewKind::WorkWeek, d(2026, 11, 2)), "November 2–6, 2026");
        assert_eq!(t(ViewKind::Week, d(2026, 10, 19)), "October 18–24, 2026");
        assert_eq!(t(ViewKind::Week, d(2026, 11, 1)), "November 1–7, 2026");
        assert_eq!(t(ViewKind::Week, d(2026, 10, 27)), "October 25–31, 2026");
        assert_eq!(range_title(ViewKind::Week, d(2026, 10, 29), &visible_days(ViewKind::Week, d(2026, 10, 29), Mon, &WORK)), "Oct 26 – Nov 1, 2026");
        assert_eq!(t(ViewKind::WorkWeek, d(2026, 12, 30)), "Dec 28, 2026 – Jan 1, 2027");
        assert_eq!(t(ViewKind::Month, d(2026, 10, 19)), "October 2026");
        assert_eq!(
            announce_text(ViewKind::Week, d(2026, 10, 19), &visible_days(ViewKind::Week, d(2026, 10, 19), Sun, &WORK)),
            "Week of October 18 to 24, 2026"
        );
        assert_eq!(
            grid_label(ViewKind::Week, d(2026, 10, 19), &visible_days(ViewKind::Week, d(2026, 10, 19), Sun, &WORK)),
            "Calendar, week of October 18, 2026"
        );
    }

    #[test]
    fn week_numbers_first_day_of_year() {
        assert_eq!(week_number(d(2026, 1, 1), Sun), 1);
        assert_eq!(week_number(d(2026, 1, 3), Sun), 1);
        assert_eq!(week_number(d(2026, 1, 4), Sun), 2);
        assert_eq!(week_number(d(2026, 10, 19), Sun), 43);
        // The week of Dec 27, 2026 contains Jan 1, 2027: week 1.
        assert_eq!(week_number(d(2026, 12, 29), Sun), 1);
        assert_eq!(week_number(d(2026, 12, 26), Sun), 52);
    }
}
