//! Expanding recurrence rules into concrete dates, and describing them.
//!
//! Rules are evaluated on local calendar dates (the system time zone), which
//! matches how Outlook shows a series to the person who owns it.

use chrono::{Datelike, Days, Duration, Months, NaiveDate, Weekday};

use crate::model::{Recurrence, RecurrenceEnd, RecurrenceFreq};

/// Hard cap on generated candidates, so a bad rule can never hang the UI.
const MAX_ITERATIONS: usize = 20_000;

/// en-US week order, used to list weekdays in descriptions.
const WEEKDAYS_FROM_SUNDAY: [Weekday; 7] =
    [Weekday::Sun, Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat];

pub fn days_in_month(year: i32, month: u32) -> u32 {
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
    let next = first.checked_add_months(Months::new(1)).expect("in range");
    (next - first).num_days() as u32
}

/// The `n`th `weekday` of a month (n = 1..=5, or -1 for the last one).
pub fn nth_weekday_of_month(year: i32, month: u32, n: i32, weekday: Weekday) -> Option<NaiveDate> {
    if n == -1 {
        let last = NaiveDate::from_ymd_opt(year, month, days_in_month(year, month))?;
        let back = (last.weekday().num_days_from_monday() + 7 - weekday.num_days_from_monday()) % 7;
        return last.checked_sub_days(Days::new(back as u64));
    }
    if !(1..=5).contains(&n) {
        return None;
    }
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let fwd = (weekday.num_days_from_monday() + 7 - first.weekday().num_days_from_monday()) % 7;
    let d = first.checked_add_days(Days::new(fwd as u64 + 7 * (n as u64 - 1)))?;
    (d.month() == month).then_some(d)
}

fn week_start_of(date: NaiveDate, week_start: Weekday) -> NaiveDate {
    let back = (date.weekday().num_days_from_monday() + 7 - week_start.num_days_from_monday()) % 7;
    date - Duration::days(back as i64)
}

fn day_in_month(year: i32, month: u32, rule: &Recurrence, fallback_day: u32) -> Option<NaiveDate> {
    if let Some((n, wd)) = rule.nth_weekday {
        return nth_weekday_of_month(year, month, n, wd);
    }
    let day = rule.month_day.unwrap_or(fallback_day).clamp(1, 31);
    NaiveDate::from_ymd_opt(year, month, day.min(days_in_month(year, month)))
}

/// All occurrence dates of `rule` for a series starting on `start`, that fall
/// within `[from, to]` (inclusive), skipping `exdates`. Occurrences before
/// `from` still count towards a `Count` end.
pub fn occurrence_dates(
    rule: &Recurrence,
    start: NaiveDate,
    week_start: Weekday,
    exdates: &[NaiveDate],
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<NaiveDate> {
    let interval = rule.interval.max(1);
    let mut out = Vec::new();
    let mut produced: u32 = 0;
    let limit_count = match rule.end {
        RecurrenceEnd::Count(n) => Some(n),
        _ => None,
    };
    let until = match rule.end {
        RecurrenceEnd::Until(d) => Some(d),
        _ => None,
    };
    let mut emit = |d: NaiveDate, out: &mut Vec<NaiveDate>| -> bool {
        // returns false when the series is finished
        if d < start {
            return true;
        }
        if until.is_some_and(|u| d > u) || d > to {
            return false;
        }
        if limit_count.is_some_and(|n| produced >= n) {
            return false;
        }
        produced += 1;
        if d >= from && !exdates.contains(&d) {
            out.push(d);
        }
        true
    };

    match rule.freq {
        RecurrenceFreq::Daily => {
            let mut d = start;
            for _ in 0..MAX_ITERATIONS {
                if !emit(d, &mut out) {
                    break;
                }
                d = match d.checked_add_days(Days::new(interval as u64)) {
                    Some(n) => n,
                    None => break,
                };
            }
        }
        RecurrenceFreq::Weekly => {
            let mut days: Vec<Weekday> = if rule.by_weekday.is_empty() { vec![start.weekday()] } else { rule.by_weekday.clone() };
            // Order days within a week starting at `week_start`.
            days.sort_by_key(|w| (w.num_days_from_monday() + 7 - week_start.num_days_from_monday()) % 7);
            days.dedup();
            let mut week = week_start_of(start, week_start);
            'outer: for _ in 0..MAX_ITERATIONS / 7 {
                for wd in &days {
                    let offset = (wd.num_days_from_monday() + 7 - week_start.num_days_from_monday()) % 7;
                    let d = week + Duration::days(offset as i64);
                    if !emit(d, &mut out) {
                        break 'outer;
                    }
                }
                week += Duration::weeks(interval as i64);
            }
        }
        RecurrenceFreq::Monthly => {
            let first_month = NaiveDate::from_ymd_opt(start.year(), start.month(), 1).expect("valid");
            for k in 0..MAX_ITERATIONS as u32 {
                let Some(m) = first_month.checked_add_months(Months::new(k * interval)) else { break };
                if m > to {
                    break;
                }
                if let Some(d) = day_in_month(m.year(), m.month(), rule, start.day())
                    && !emit(d, &mut out) {
                        break;
                    }
            }
        }
        RecurrenceFreq::Yearly => {
            let month = rule.month.unwrap_or(start.month()).clamp(1, 12);
            for k in 0..(MAX_ITERATIONS as i32) {
                let year = start.year() + k * interval as i32;
                if NaiveDate::from_ymd_opt(year, 1, 1).is_none_or(|y| y > to) {
                    break;
                }
                if let Some(d) = day_in_month(year, month, rule, start.day())
                    && !emit(d, &mut out) {
                        break;
                    }
            }
        }
    }
    out
}

/// "Monday".
pub fn weekday_name(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// "first" … "fourth"; anything else (-1, 5) reads as "last".
pub fn ordinal_name(n: i32) -> &'static str {
    match n {
        1 => "first",
        2 => "second",
        3 => "third",
        4 => "fourth",
        _ => "last",
    }
}

/// Which "Nth weekday" of its month a date is: (1..=5, is it the last one).
pub fn nth_of_month(date: NaiveDate) -> (i32, bool) {
    let n = ((date.day() - 1) / 7 + 1) as i32;
    let last = date.day() + 7 > days_in_month(date.year(), date.month());
    (n, last)
}

fn join_names(parts: &[&str]) -> String {
    match parts.len() {
        0 => String::new(),
        1 => parts[0].to_string(),
        _ => format!("{} and {}", parts[..parts.len() - 1].join(", "), parts[parts.len() - 1]),
    }
}

fn is_every_weekday(rule: &Recurrence) -> bool {
    use Weekday::*;
    let mut days = rule.by_weekday.clone();
    days.sort_by_key(|w| w.num_days_from_monday());
    days.dedup();
    rule.freq == RecurrenceFreq::Weekly && rule.interval.max(1) == 1 && days == [Mon, Tue, Wed, Thu, Fri]
}

fn month_name(month: u32) -> String {
    NaiveDate::from_ymd_opt(2000, month.clamp(1, 12), 1).map(|d| d.format("%B").to_string()).unwrap_or_default()
}

/// The pattern part of a description, without "Occurs " or the end:
/// "every Monday and Wednesday", "every 2 weeks on Friday", "every weekday".
fn pattern_text(rule: &Recurrence, start: NaiveDate) -> String {
    let n = rule.interval.max(1);
    match rule.freq {
        RecurrenceFreq::Daily if n == 1 => "every day".to_string(),
        RecurrenceFreq::Daily => format!("every {n} days"),
        RecurrenceFreq::Weekly if is_every_weekday(rule) => "every weekday".to_string(),
        RecurrenceFreq::Weekly => {
            let days: Vec<Weekday> = if rule.by_weekday.is_empty() { vec![start.weekday()] } else { rule.by_weekday.clone() };
            let names: Vec<&str> = WEEKDAYS_FROM_SUNDAY.iter().filter(|w| days.contains(w)).map(|w| weekday_name(*w)).collect();
            let names = join_names(&names);
            if n == 1 { format!("every {names}") } else { format!("every {n} weeks on {names}") }
        }
        RecurrenceFreq::Monthly => {
            let every = if n == 1 { "every month".to_string() } else { format!("every {n} months") };
            match rule.nth_weekday {
                Some((k, wd)) => format!("{every} on the {} {}", ordinal_name(k), weekday_name(wd)),
                None => format!("{every} on day {}", rule.month_day.unwrap_or(start.day())),
            }
        }
        RecurrenceFreq::Yearly => {
            let month = month_name(rule.month.unwrap_or(start.month()));
            let every = if n == 1 { "every year".to_string() } else { format!("every {n} years") };
            match rule.nth_weekday {
                Some((k, wd)) => format!("{every} on the {} {} of {month}", ordinal_name(k), weekday_name(wd)),
                None => format!("{every} on {month} {}", rule.month_day.unwrap_or(start.day())),
            }
        }
    }
}

/// Human description in Outlook's style, e.g. "Occurs every Monday and Wednesday".
pub fn describe(rule: &Recurrence, start: NaiveDate) -> String {
    let base = format!("Occurs {}", pattern_text(rule, start));
    match rule.end {
        RecurrenceEnd::Never => base,
        RecurrenceEnd::Until(d) => format!("{base} until {}", d.format("%-m/%-d/%Y")),
        RecurrenceEnd::Count(c) => format!("{base}, {c} times"),
    }
}

/// Summary line of the Repeat dialog: "Occurs every 2 weeks on Monday and
/// Wednesday until December 21, 2026".
pub fn describe_long(rule: &Recurrence, start: NaiveDate) -> String {
    let base = format!("Occurs {}", pattern_text(rule, start));
    match rule.end {
        RecurrenceEnd::Never => base,
        RecurrenceEnd::Until(d) => format!("{base} until {}", d.format("%B %-d, %Y")),
        RecurrenceEnd::Count(c) => format!("{base}, {c} times"),
    }
}

/// Resting label of the editor's Repeat button for a series:
/// "Every Monday until 12/21/2026".
pub fn short_label(rule: &Recurrence, start: NaiveDate) -> String {
    let text = pattern_text(rule, start);
    let mut chars = text.chars();
    let base = match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    };
    match rule.end {
        RecurrenceEnd::Never => base,
        RecurrenceEnd::Until(d) => format!("{base} until {}", d.format("%-m/%-d/%Y")),
        RecurrenceEnd::Count(c) => format!("{base}, {c} times"),
    }
}

/// The Repeat dropdown's presets for a series starting on `start`, in
/// Outlook's order, as (label, rule). The UI adds "Don't repeat" before and
/// "Custom…" after. `work_days` drives "Every weekday".
pub fn presets(start: NaiveDate, work_days: &[Weekday]) -> Vec<(String, Recurrence)> {
    let base = Recurrence { interval: 1, ..Default::default() };
    let wd = start.weekday();
    let (n, last) = nth_of_month(start);
    let mut work: Vec<Weekday> = WEEKDAYS_FROM_SUNDAY.iter().copied().filter(|w| work_days.contains(w)).collect();
    if work.is_empty() {
        work = vec![Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri];
    }
    let mut out = vec![
        ("Daily".to_string(), Recurrence { freq: RecurrenceFreq::Daily, ..base.clone() }),
        ("Every weekday".to_string(), Recurrence { freq: RecurrenceFreq::Weekly, by_weekday: work, ..base.clone() }),
        (
            format!("Weekly on {}", weekday_name(wd)),
            Recurrence { freq: RecurrenceFreq::Weekly, by_weekday: vec![wd], ..base.clone() },
        ),
    ];
    if n <= 4 {
        out.push((
            format!("Monthly on the {} {}", ordinal_name(n), weekday_name(wd)),
            Recurrence { freq: RecurrenceFreq::Monthly, nth_weekday: Some((n, wd)), ..base.clone() },
        ));
    }
    if last {
        out.push((
            format!("Monthly on the last {}", weekday_name(wd)),
            Recurrence { freq: RecurrenceFreq::Monthly, nth_weekday: Some((-1, wd)), ..base.clone() },
        ));
    }
    out.push((
        format!("Monthly on day {}", start.day()),
        Recurrence { freq: RecurrenceFreq::Monthly, month_day: Some(start.day()), ..base.clone() },
    ));
    out.push((
        format!("Yearly on {} {}", month_name(start.month()), start.day()),
        Recurrence { freq: RecurrenceFreq::Yearly, month: Some(start.month()), month_day: Some(start.day()), ..base },
    ));
    out
}

/// True when two rules describe the same series (weekday order ignored).
/// Used to tick the matching preset in the Repeat dropdown.
pub fn same_pattern(a: &Recurrence, b: &Recurrence) -> bool {
    let norm = |r: &Recurrence| {
        let mut days = r.by_weekday.clone();
        days.sort_by_key(|w| w.num_days_from_monday());
        days.dedup();
        (r.freq, r.interval.max(1), days, r.month_day, r.nth_weekday, r.month, r.end)
    };
    norm(a) == norm(b)
}

/// How many occurrences the rule generates before `date` (deleted ones
/// included, as RFC 5545 COUNT does). Used when splitting a numbered series.
pub fn count_before(rule: &Recurrence, start: NaiveDate, week_start: Weekday, date: NaiveDate) -> u32 {
    if date <= start {
        return 0;
    }
    let open = Recurrence { end: RecurrenceEnd::Never, ..rule.clone() };
    let n = occurrence_dates(&open, start, week_start, &[], start, date - Duration::days(1)).len() as u32;
    match rule.end {
        RecurrenceEnd::Count(c) => n.min(c),
        _ => n,
    }
}

/// Whether the series has a (not deleted) occurrence on `date`.
pub fn occurs_on(rule: &Recurrence, start: NaiveDate, week_start: Weekday, exdates: &[NaiveDate], date: NaiveDate) -> bool {
    !occurrence_dates(rule, start, week_start, exdates, date, date).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Weekday::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn rule(freq: RecurrenceFreq) -> Recurrence {
        Recurrence { freq, interval: 1, ..Default::default() }
    }

    #[test]
    fn weekly_multiple_days_with_count() {
        let mut r = rule(RecurrenceFreq::Weekly);
        r.by_weekday = vec![Wed, Mon];
        r.end = RecurrenceEnd::Count(4);
        // Series starts Mon 2026-10-19.
        let got = occurrence_dates(&r, d(2026, 10, 19), Sun, &[], d(2026, 10, 1), d(2026, 12, 31));
        assert_eq!(got, vec![d(2026, 10, 19), d(2026, 10, 21), d(2026, 10, 26), d(2026, 10, 28)]);
    }

    #[test]
    fn biweekly_respects_interval_and_exdates() {
        let mut r = rule(RecurrenceFreq::Weekly);
        r.interval = 2;
        let got = occurrence_dates(&r, d(2026, 10, 20), Sun, &[d(2026, 11, 3)], d(2026, 10, 1), d(2026, 11, 30));
        assert_eq!(got, vec![d(2026, 10, 20), d(2026, 11, 17)]);
    }

    #[test]
    fn monthly_clamps_day_31_and_nth_weekday() {
        let r = rule(RecurrenceFreq::Monthly);
        let got = occurrence_dates(&r, d(2026, 1, 31), Sun, &[], d(2026, 1, 1), d(2026, 4, 30));
        assert_eq!(got, vec![d(2026, 1, 31), d(2026, 2, 28), d(2026, 3, 31), d(2026, 4, 30)]);
        let mut r2 = rule(RecurrenceFreq::Monthly);
        r2.nth_weekday = Some((-1, Fri));
        let got = occurrence_dates(&r2, d(2026, 10, 1), Sun, &[], d(2026, 10, 1), d(2026, 12, 31));
        assert_eq!(got, vec![d(2026, 10, 30), d(2026, 11, 27), d(2026, 12, 25)]);
        assert_eq!(nth_weekday_of_month(2026, 10, 2, Tue), Some(d(2026, 10, 13)));
        assert_eq!(nth_weekday_of_month(2026, 2, 5, Mon), None);
    }

    #[test]
    fn daily_until_and_range_window() {
        let mut r = rule(RecurrenceFreq::Daily);
        r.end = RecurrenceEnd::Until(d(2026, 10, 25));
        let got = occurrence_dates(&r, d(2026, 10, 19), Sun, &[], d(2026, 10, 23), d(2026, 10, 31));
        assert_eq!(got, vec![d(2026, 10, 23), d(2026, 10, 24), d(2026, 10, 25)]);
    }

    #[test]
    fn yearly_feb_29_falls_back_to_28() {
        let r = rule(RecurrenceFreq::Yearly);
        let got = occurrence_dates(&r, d(2028, 2, 29), Sun, &[], d(2028, 1, 1), d(2030, 12, 31));
        assert_eq!(got, vec![d(2028, 2, 29), d(2029, 2, 28), d(2030, 2, 28)]);
    }

    #[test]
    fn yearly_nth_weekday_of_month() {
        let mut r = rule(RecurrenceFreq::Yearly);
        r.month = Some(11);
        r.nth_weekday = Some((4, Thu));
        let got = occurrence_dates(&r, d(2026, 11, 26), Sun, &[], d(2026, 1, 1), d(2028, 12, 31));
        assert_eq!(got, vec![d(2026, 11, 26), d(2027, 11, 25), d(2028, 11, 23)]);
    }

    #[test]
    fn describe_reads_naturally() {
        let mut r = rule(RecurrenceFreq::Weekly);
        r.by_weekday = vec![Mon, Wed];
        assert_eq!(describe(&r, d(2026, 10, 19)), "Occurs every Monday and Wednesday");
        let mut m = rule(RecurrenceFreq::Monthly);
        m.nth_weekday = Some((2, Tue));
        m.end = RecurrenceEnd::Count(5);
        assert_eq!(describe(&m, d(2026, 10, 13)), "Occurs every month on the second Tuesday, 5 times");
    }

    #[test]
    fn descriptions_for_editor_and_dialog() {
        let mut r = rule(RecurrenceFreq::Weekly);
        r.interval = 2;
        r.by_weekday = vec![Wed, Mon];
        r.end = RecurrenceEnd::Until(d(2026, 12, 21));
        assert_eq!(
            describe_long(&r, d(2026, 10, 19)),
            "Occurs every 2 weeks on Monday and Wednesday until December 21, 2026"
        );
        let mut weekly = rule(RecurrenceFreq::Weekly);
        weekly.end = RecurrenceEnd::Until(d(2026, 12, 21));
        assert_eq!(short_label(&weekly, d(2026, 10, 19)), "Every Monday until 12/21/2026");
        let mut weekdays = rule(RecurrenceFreq::Weekly);
        weekdays.by_weekday = vec![Mon, Tue, Wed, Thu, Fri];
        assert_eq!(describe(&weekdays, d(2026, 10, 19)), "Occurs every weekday");
        let y = Recurrence { freq: RecurrenceFreq::Yearly, interval: 1, month: Some(10), month_day: Some(19), ..Default::default() };
        assert_eq!(describe(&y, d(2026, 10, 19)), "Occurs every year on October 19");
    }

    #[test]
    fn presets_follow_the_start_date() {
        // Monday, October 19, 2026 is the third Monday and not the last one.
        let labels: Vec<String> = presets(d(2026, 10, 19), &[Mon, Tue, Wed, Thu, Fri]).into_iter().map(|p| p.0).collect();
        assert_eq!(
            labels,
            [
                "Daily",
                "Every weekday",
                "Weekly on Monday",
                "Monthly on the third Monday",
                "Monthly on day 19",
                "Yearly on October 19"
            ]
        );
        // Friday, October 30, 2026 is the fifth and last Friday: no "fifth" preset.
        let labels: Vec<String> = presets(d(2026, 10, 30), &[Mon]).into_iter().map(|p| p.0).collect();
        assert!(labels.contains(&"Monthly on the last Friday".to_string()));
        assert!(!labels.iter().any(|l| l.contains("fifth")));
        // Monday, October 26 is the fourth and also the last Monday.
        let labels: Vec<String> = presets(d(2026, 10, 26), &[Mon]).into_iter().map(|p| p.0).collect();
        assert!(labels.contains(&"Monthly on the fourth Monday".to_string()));
        assert!(labels.contains(&"Monthly on the last Monday".to_string()));
        let every_weekday = &presets(d(2026, 10, 19), &[Mon, Tue, Wed, Thu, Fri])[1].1;
        let mut shuffled = every_weekday.clone();
        shuffled.by_weekday.reverse();
        assert!(same_pattern(every_weekday, &shuffled));
    }

    #[test]
    fn counting_and_membership() {
        let mut r = rule(RecurrenceFreq::Weekly);
        r.end = RecurrenceEnd::Count(10);
        let start = d(2026, 10, 5);
        assert_eq!(count_before(&r, start, Sun, d(2026, 10, 19)), 2);
        assert_eq!(count_before(&r, start, Sun, start), 0);
        assert_eq!(count_before(&r, start, Sun, d(2027, 6, 1)), 10);
        assert!(occurs_on(&r, start, Sun, &[], d(2026, 10, 12)));
        assert!(!occurs_on(&r, start, Sun, &[d(2026, 10, 12)], d(2026, 10, 12)));
        assert!(!occurs_on(&r, start, Sun, &[], d(2026, 10, 13)));
        assert_eq!(nth_of_month(d(2026, 10, 19)), (3, false));
        assert_eq!(nth_of_month(d(2026, 10, 30)), (5, true));
    }
}
