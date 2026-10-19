//! Compact month date picker (APG date-picker grid keys): arrows by day and
//! week, PageUp/PageDown by month, Enter picks, Escape closes.

use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};
use dioxus::prelude::*;

use crate::kit::{IconButton, dom};

/// The 42 days shown for `month` (first day of a month) with weeks
/// starting on `week_start`.
pub fn month_grid(month: NaiveDate, week_start: Weekday) -> Vec<NaiveDate> {
    let first = month.with_day(1).unwrap_or(month);
    let offset = (first.weekday().num_days_from_monday() + 7 - week_start.num_days_from_monday()) % 7;
    let start = first - Duration::days(offset as i64);
    (0..42).map(|i| start + Duration::days(i)).collect()
}

#[component]
pub fn MiniMonth(selected: NaiveDate, today: NaiveDate, week_start: Weekday, on_pick: EventHandler<NaiveDate>, on_close: EventHandler<()>) -> Element {
    let mut month = use_signal(|| selected.with_day(1).unwrap_or(selected));
    let id = dom::use_unique_id("ax-minimonth");
    let m = month();
    let days = month_grid(m, week_start);
    let heads: Vec<String> = days.iter().take(7).map(|d| d.format("%a").to_string().chars().take(2).collect()).collect();
    let title = m.format("%B %Y").to_string();
    let mut shift = move |n: i32| {
        let cur = month();
        let next = if n >= 0 { cur.checked_add_months(Months::new(n as u32)) } else { cur.checked_sub_months(Months::new((-n) as u32)) };
        month.set(next.unwrap_or(cur));
    };
    rsx! {
        div {
            id: "{id}",
            class: "ax-minimonth",
            role: "dialog",
            "aria-label": "Choose a date",
            onmounted: {
                let id = id.clone();
                move |_| dom::open_popup(&id)
            },
            onkeydown: move |e: KeyboardEvent| match e.key() {
                Key::Escape => {
                    e.stop_propagation();
                    on_close.call(());
                }
                Key::PageUp => {
                    e.prevent_default();
                    shift(if e.modifiers().shift() { -12 } else { -1 });
                }
                Key::PageDown => {
                    e.prevent_default();
                    shift(if e.modifiers().shift() { 12 } else { 1 });
                }
                _ => {}
            },
            div { class: "ax-minimonth-head",
                span { class: "ax-semibold", "aria-live": "polite", "{title}" }
                span { class: "ax-grow" }
                IconButton { icon: "arrow_up", title: "Previous month", small: true, onclick: move |_| shift(-1) }
                IconButton { icon: "arrow_down", title: "Next month", small: true, onclick: move |_| shift(1) }
            }
            div { class: "ax-minimonth-grid", role: "grid", "aria-label": "{title}", "data-ax-roving": "grid", "data-ax-cols": "7",
                for h in heads {
                    span { class: "ax-minimonth-dow", role: "columnheader", "{h}" }
                }
                for d in days {
                    button {
                        key: "{d}",
                        r#type: "button",
                        role: "gridcell",
                        class: "ax-minimonth-day",
                        class: if d.month() != m.month() { "ax-outside" },
                        class: if d == today { "ax-today" },
                        class: if d == selected { "ax-selected" },
                        "aria-selected": if d == selected { "true" } else { "false" },
                        "aria-current": if d == today { "date" },
                        "aria-label": "{crate::fmt::long_date(d)}",
                        "data-ax-item": "true",
                        tabindex: if d == selected { "0" } else { "-1" },
                        onclick: move |_| on_pick.call(d),
                        "{d.day()}"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_starts_on_the_week_start() {
        let oct = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(); // a Thursday
        let sun = month_grid(oct, Weekday::Sun);
        assert_eq!(sun[0], NaiveDate::from_ymd_opt(2026, 9, 27).unwrap());
        assert_eq!(sun.len(), 42);
        let mon = month_grid(oct, Weekday::Mon);
        assert_eq!(mon[0], NaiveDate::from_ymd_opt(2026, 9, 28).unwrap());
    }
}
