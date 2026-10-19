//! Sorting (spec 3.4) and Planned grouping (spec 3.11).

use std::cmp::Ordering;

use axentra_core::*;
use chrono::{Datelike, Duration, NaiveDate, Weekday};

use super::ctx::SortKey;

/// Sort `tasks` in place. The sort is stable, so ties keep the incoming
/// (manual) order. `reverse` flips the primary key only; tasks without a due
/// date stay last either way.
pub fn sort_tasks(tasks: &mut [Task], key: SortKey, reverse: bool) {
    let flip = |o: Ordering| if reverse { o.reverse() } else { o };
    match key {
        SortKey::Importance => tasks.sort_by(|a, b| flip(b.is_important.cmp(&a.is_important))),
        SortKey::DueDate => tasks.sort_by(|a, b| match (a.due, b.due) {
            (Some(x), Some(y)) => flip(x.cmp(&y)).then(b.is_important.cmp(&a.is_important)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => b.is_important.cmp(&a.is_important),
        }),
        SortKey::MyDay => tasks.sort_by(|a, b| {
            let in_a = a.my_day.is_some();
            let in_b = b.my_day.is_some();
            flip(in_b.cmp(&in_a).then(b.my_day_at.cmp(&a.my_day_at)))
        }),
        SortKey::Alphabetical => tasks.sort_by(|a, b| {
            flip(a.title.to_lowercase().cmp(&b.title.to_lowercase())).then(a.created_at.cmp(&b.created_at))
        }),
        SortKey::Created => tasks.sort_by(|a, b| flip(b.created_at.cmp(&a.created_at)).then(a.title.cmp(&b.title))),
    }
}

/// "Move starred tasks to top": stable partition, important first.
pub fn starred_first(tasks: &mut [Task]) {
    tasks.sort_by_key(|t| !t.is_important);
}

/// Planned sections in display order: (key, heading).
pub const PLANNED_SECTIONS: [(&str, &str); 6] = [
    ("earlier", "Earlier"),
    ("today", "Today"),
    ("tomorrow", "Tomorrow"),
    ("week", "This week"),
    ("later", "Later"),
    ("reminder", "No due date"),
];

/// Which Planned section a task falls in. "This week" runs to the end of the
/// week that contains today (`week_start` is the first day of the week).
pub fn planned_section(t: &Task, today: NaiveDate, week_start: Weekday) -> &'static str {
    let Some(due) = t.due else { return "reminder" };
    let into_week = (today.weekday().num_days_from_monday() + 7 - week_start.num_days_from_monday()) % 7;
    let week_end = today + Duration::days(6 - into_week as i64);
    match (due - today).num_days() {
        d if d < 0 => "earlier",
        0 => "today",
        1 => "tomorrow",
        _ if due <= week_end => "week",
        _ => "later",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn task(title: &str) -> Task {
        Task { title: title.into(), ..Default::default() }
    }

    fn titles(tasks: &[Task]) -> Vec<&str> {
        tasks.iter().map(|t| t.title.as_str()).collect()
    }

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    #[test]
    fn sorts() {
        let mut v = vec![
            Task { due: Some(d(25)), created_at: Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(), ..task("beta") },
            Task { is_important: true, created_at: Utc.with_ymd_and_hms(2026, 10, 3, 0, 0, 0).unwrap(), ..task("Alpha") },
            Task { due: Some(d(20)), my_day: Some(d(19)), created_at: Utc.with_ymd_and_hms(2026, 10, 2, 0, 0, 0).unwrap(), ..task("gamma") },
        ];
        sort_tasks(&mut v, SortKey::Importance, false);
        assert_eq!(titles(&v), ["Alpha", "beta", "gamma"]);
        sort_tasks(&mut v, SortKey::DueDate, false);
        assert_eq!(titles(&v), ["gamma", "beta", "Alpha"]);
        sort_tasks(&mut v, SortKey::DueDate, true);
        assert_eq!(titles(&v), ["beta", "gamma", "Alpha"], "no due date stays last");
        sort_tasks(&mut v, SortKey::Alphabetical, false);
        assert_eq!(titles(&v), ["Alpha", "beta", "gamma"]);
        sort_tasks(&mut v, SortKey::Alphabetical, true);
        assert_eq!(titles(&v), ["gamma", "beta", "Alpha"]);
        sort_tasks(&mut v, SortKey::Created, false);
        assert_eq!(titles(&v), ["Alpha", "gamma", "beta"]);
        sort_tasks(&mut v, SortKey::MyDay, false);
        assert_eq!(titles(&v)[0], "gamma");
        starred_first(&mut v);
        assert_eq!(titles(&v)[0], "Alpha");
    }

    #[test]
    fn planned_sections() {
        let today = d(19); // Monday
        let at = |day: Option<u32>| planned_section(&Task { due: day.map(d), ..task("x") }, today, Weekday::Sun);
        assert_eq!(at(Some(17)), "earlier");
        assert_eq!(at(Some(19)), "today");
        assert_eq!(at(Some(20)), "tomorrow");
        assert_eq!(at(Some(24)), "week", "Saturday ends a Sunday-start week");
        assert_eq!(at(Some(25)), "later");
        assert_eq!(at(None), "reminder");
        assert_eq!(planned_section(&Task { due: Some(d(25)), ..task("x") }, today, Weekday::Mon), "week");
    }
}
