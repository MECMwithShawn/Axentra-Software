//! Mini month (date navigator): the folder pane's month grid, also used by
//! the title flyout and the editor's date pickers. A roving-tabindex grid:
//! arrows move by day / week, PageUp/PageDown by month (Shift: year),
//! Home/End to the week's ends, Enter/Space picks.

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use dioxus::prelude::*;

use super::range::{self, add_months, first_of_month, week_number, week_start_of};
use crate::kit::IconButton;

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[component]
pub fn MiniMonth(
    /// Any date in the month shown.
    month: NaiveDate,
    on_month: EventHandler<NaiveDate>,
    on_pick: EventHandler<NaiveDate>,
    today: NaiveDate,
    week_start: Weekday,
    /// Days banded as the visible range.
    #[props(default)]
    range: Vec<NaiveDate>,
    /// The single selected day (outlined).
    #[props(default)]
    selected: Option<NaiveDate>,
    #[props(default)] week_numbers: bool,
    on_week: Option<EventHandler<NaiveDate>>,
) -> Element {
    let id = use_hook(|| NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let month = first_of_month(month);
    let mut focus = use_signal(|| selected.unwrap_or(today));
    // Keep the roving focus inside the shown month.
    let focus_day = if first_of_month(focus()) == month { focus() } else { selected.filter(|d| first_of_month(*d) == month).unwrap_or(month) };
    let cells = range::mini_month_grid(month, week_start);
    let letters: Vec<String> = (0..7)
        .map(|i| (week_start_of(month, week_start) + Duration::days(i)).format("%a").to_string().chars().next().unwrap_or(' ').to_string())
        .collect();
    let in_range = |d: NaiveDate| range.contains(&d);
    let mut move_focus = move |d: NaiveDate| {
        focus.set(d);
        if first_of_month(d) != month {
            on_month.call(first_of_month(d));
        }
        document::eval(&format!(
            "setTimeout(()=>{{const el=document.querySelector('[data-mini=\"{id}\"] [data-date=\"{d}\"]');if(el)el.focus();}},0)"
        ));
    };
    let title = month.format("%B %Y").to_string();
    rsx! {
        div { class: "ax-cal-mini", "data-mini": "{id}",
            div { class: "ax-cal-mini-head",
                span { class: "ax-cal-mini-title", "{title}" }
                IconButton { icon: "arrow_up", title: "Go to previous month", small: true, onclick: move |_| on_month.call(add_months(month, -1)) }
                IconButton { icon: "arrow_down", title: "Go to next month", small: true, onclick: move |_| on_month.call(add_months(month, 1)) }
            }
            div {
                class: "ax-cal-mini-grid",
                class: if week_numbers { "ax-with-weeknums" },
                role: "grid",
                "aria-label": "{title}",
                onkeydown: move |e: KeyboardEvent| {
                    let shift = e.modifiers().shift();
                    let d = focus_day;
                    let next = match e.key() {
                        Key::ArrowLeft => Some(d - Duration::days(1)),
                        Key::ArrowRight => Some(d + Duration::days(1)),
                        Key::ArrowUp => Some(d - Duration::days(7)),
                        Key::ArrowDown => Some(d + Duration::days(7)),
                        Key::PageUp => Some(add_months(d, if shift { -12 } else { -1 })),
                        Key::PageDown => Some(add_months(d, if shift { 12 } else { 1 })),
                        Key::Home => Some(week_start_of(d, week_start)),
                        Key::End => Some(week_start_of(d, week_start) + Duration::days(6)),
                        Key::Enter => {
                            e.prevent_default();
                            e.stop_propagation();
                            on_pick.call(d);
                            None
                        }
                        Key::Character(ref c) if c == " " => {
                            e.prevent_default();
                            e.stop_propagation();
                            on_pick.call(d);
                            None
                        }
                        _ => None,
                    };
                    if let Some(n) = next {
                        e.prevent_default();
                        e.stop_propagation();
                        move_focus(n);
                    }
                },
                div { class: "ax-cal-mini-row ax-cal-mini-letters", role: "row",
                    if week_numbers { span { class: "ax-cal-mini-wk" } }
                    for (i, l) in letters.iter().enumerate() {
                        span { key: "{i}", class: "ax-cal-mini-letter", role: "columnheader", "{l}" }
                    }
                }
                for week in cells.chunks(7) {
                    div { key: "{week[0]}", class: "ax-cal-mini-row", role: "row",
                        if week_numbers {
                            {
                                let d0 = week[0];
                                let n = week_number(d0, week_start);
                                rsx! {
                                    button {
                                        r#type: "button",
                                        class: "ax-cal-mini-wk",
                                        tabindex: "-1",
                                        title: "Go to week {n}",
                                        disabled: on_week.is_none(),
                                        onclick: move |_| if let Some(h) = &on_week { h.call(d0) },
                                        "{n}"
                                    }
                                }
                            }
                        }
                        for (i, d) in week.iter().copied().enumerate() {
                            {
                                let banded = in_range(d);
                                let start = banded && (i == 0 || !in_range(d - Duration::days(1)));
                                let end = banded && (i == 6 || !in_range(d + Duration::days(1)));
                                rsx! {
                                    button {
                                        key: "{d}",
                                        r#type: "button",
                                        class: "ax-cal-mini-day",
                                        class: if d == today { "ax-today" },
                                        class: if d.month() != month.month() { "ax-outside" },
                                        class: if banded { "ax-in-range" },
                                        class: if start { "ax-range-start" },
                                        class: if end { "ax-range-end" },
                                        class: if selected == Some(d) { "ax-selected" },
                                        role: "gridcell",
                                        "data-date": "{d}",
                                        tabindex: if d == focus_day { "0" } else { "-1" },
                                        "aria-selected": if selected == Some(d) || banded { "true" } else { "false" },
                                        "aria-current": if d == today { "date" } else { "false" },
                                        "aria-label": d.format("%A, %B %-d, %Y").to_string(),
                                        onclick: move |_| {
                                            focus.set(d);
                                            on_pick.call(d);
                                        },
                                        span { {d.day().to_string()} }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
