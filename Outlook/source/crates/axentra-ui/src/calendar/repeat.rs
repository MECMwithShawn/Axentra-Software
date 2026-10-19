//! Repeat: the editor's dropdown of presets and the custom Repeat dialog
//! (interval, weekdays, monthly / yearly variants, end after a date or a
//! number of occurrences).

use axentra_core::recurrence::{describe_long, nth_of_month, ordinal_name, presets, same_pattern, short_label, weekday_name};
use axentra_core::*;
use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};
use dioxus::prelude::*;

use super::fields::DateField;
use super::range::week_start_of;
use crate::kit::{Button, Dialog, MenuButton, MenuItem, Variant, keys};

/// Resting label of the Repeat button.
pub fn repeat_label(rule: Option<&Recurrence>, start: NaiveDate) -> String {
    match rule {
        None => "Don't repeat".into(),
        Some(r) => short_label(r, start),
    }
}

/// The Repeat dropdown: "Don't repeat", the presets for the start date,
/// then "Custom…" (opens the dialog through `on_custom`).
#[component]
pub fn RepeatMenu(
    rule: Option<Recurrence>,
    start: NaiveDate,
    work_days: Vec<Weekday>,
    onchange: EventHandler<Option<Recurrence>>,
    on_custom: EventHandler<()>,
    #[props(default)] disabled: bool,
) -> Element {
    let list = presets(start, &work_days);
    let mut items = vec![MenuItem::new("none", "Don't repeat").checked(rule.is_none())];
    for (i, (label, r)) in list.iter().enumerate() {
        items.push(MenuItem::new(format!("p{i}"), label.clone()).checked(rule.as_ref().is_some_and(|x| same_pattern(x, r))));
    }
    items.push(MenuItem::divider());
    let custom = rule.as_ref().is_some_and(|x| !list.iter().any(|(_, r)| same_pattern(x, r)));
    items.push(MenuItem::new("custom", "Custom…").checked(custom));
    let label = repeat_label(rule.as_ref(), start);
    rsx! {
        MenuButton {
            label,
            icon: "arrow_repeat_all",
            items,
            variant: Variant::Subtle,
            disabled,
            class: "ax-cal-repeat-btn",
            on_select: move |id: String| match id.as_str() {
                "none" => onchange.call(None),
                "custom" => on_custom.call(()),
                p => {
                    let idx = p.trim_start_matches('p').parse::<usize>().ok();
                    if let Some((_, r)) = idx.and_then(|i| list.get(i)) {
                        // Keep an existing end when switching patterns.
                        let mut r = r.clone();
                        if let Some(old) = &rule {
                            r.end = old.end;
                        }
                        onchange.call(Some(r));
                    }
                }
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MonthMode {
    Day,
    Nth,
    Last,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EndMode {
    Never,
    Until,
    Count,
}

/// Custom repeat dialog. `on_save(None)` = "Remove recurrence".
#[component]
pub fn RepeatDialog(
    rule: Option<Recurrence>,
    start: NaiveDate,
    week_start: Weekday,
    on_start: EventHandler<NaiveDate>,
    on_save: EventHandler<Option<Recurrence>>,
    on_close: EventHandler<()>,
) -> Element {
    let existing = rule.is_some();
    let base = rule.clone().unwrap_or(Recurrence {
        freq: RecurrenceFreq::Weekly,
        interval: 1,
        by_weekday: vec![start.weekday()],
        ..Default::default()
    });
    let (n, is_last) = nth_of_month(start);
    let mut freq = use_signal(|| base.freq);
    let mut interval = use_signal(|| base.interval.max(1));
    let mut days = use_signal(|| if base.by_weekday.is_empty() { vec![start.weekday()] } else { base.by_weekday.clone() });
    let mut month_mode = use_signal(|| match base.nth_weekday {
        Some((-1, _)) => MonthMode::Last,
        Some(_) => MonthMode::Nth,
        None => MonthMode::Day,
    });
    let mut end_mode = use_signal(|| match base.end {
        RecurrenceEnd::Never => EndMode::Never,
        RecurrenceEnd::Until(_) => EndMode::Until,
        RecurrenceEnd::Count(_) => EndMode::Count,
    });
    let mut until = use_signal(|| match base.end {
        RecurrenceEnd::Until(d) => d,
        _ => start + Months::new(3),
    });
    let mut count = use_signal(|| match base.end {
        RecurrenceEnd::Count(c) => c,
        _ => 10,
    });

    let build = move || -> Recurrence {
        let f = freq();
        let wd = start.weekday();
        let nth = match month_mode() {
            MonthMode::Day => None,
            MonthMode::Nth => Some((n.min(4), wd)),
            MonthMode::Last => Some((-1, wd)),
        };
        Recurrence {
            freq: f,
            interval: interval().clamp(1, 99),
            by_weekday: if f == RecurrenceFreq::Weekly { days() } else { Vec::new() },
            month_day: match f {
                RecurrenceFreq::Monthly | RecurrenceFreq::Yearly if nth.is_none() => Some(start.day()),
                _ => None,
            },
            nth_weekday: match f {
                RecurrenceFreq::Monthly | RecurrenceFreq::Yearly => nth,
                _ => None,
            },
            month: (f == RecurrenceFreq::Yearly).then_some(start.month()),
            end: match end_mode() {
                EndMode::Never => RecurrenceEnd::Never,
                EndMode::Until => RecurrenceEnd::Until(until().max(start)),
                EndMode::Count => RecurrenceEnd::Count(count().max(1)),
            },
        }
    };
    let summary = describe_long(&build(), start);
    let unit_options: Vec<(String, String)> = [("daily", "day(s)"), ("weekly", "week(s)"), ("monthly", "month(s)"), ("yearly", "year(s)")]
        .iter()
        .map(|(v, l)| (v.to_string(), l.to_string()))
        .collect();
    let unit_value = match freq() {
        RecurrenceFreq::Daily => "daily",
        RecurrenceFreq::Weekly => "weekly",
        RecurrenceFreq::Monthly => "monthly",
        RecurrenceFreq::Yearly => "yearly",
    };
    let first = week_start_of(start, week_start);
    let week: Vec<Weekday> = (0..7).map(|i| (first + Duration::days(i)).weekday()).collect();
    let wd_name = weekday_name(start.weekday());
    let month_name = start.format("%B").to_string();
    let long_month = start.day() > 28;
    rsx! {
        Dialog {
            title: "Repeat",
            width: 440,
            on_close: move |_| on_close.call(()),
            actions: rsx! {
                Button { label: "Save", variant: Variant::Primary, onclick: move |_| on_save.call(Some(build())) }
                Button { label: "Cancel", onclick: move |_| on_close.call(()) }
                if existing {
                    Button { label: "Remove recurrence", variant: Variant::Subtle, onclick: move |_| on_save.call(None) }
                }
            },
            div { class: "ax-cal-repeat",
                div { class: "ax-cal-repeat-row",
                    span { class: "ax-cal-repeat-label", "Start" }
                    DateField { value: start, label: "Start", week_start, onchange: move |d| on_start.call(d) }
                }
                div { class: "ax-cal-repeat-row",
                    span { class: "ax-cal-repeat-label", "Repeat every" }
                    input {
                        class: "ax-input ax-cal-repeat-n",
                        r#type: "number",
                        min: "1",
                        max: "99",
                        "aria-label": "Repeat every",
                        value: "{interval}",
                        oninput: move |e| if let Ok(v) = e.value().parse::<u32>() { interval.set(v.clamp(1, 99)) },
                        onkeydown: move |e| keys::stop_typing_keys(&e),
                    }
                    crate::kit::Select {
                        value: unit_value,
                        options: unit_options,
                        label: "",
                        onchange: move |v: String| freq.set(match v.as_str() {
                            "daily" => RecurrenceFreq::Daily,
                            "monthly" => RecurrenceFreq::Monthly,
                            "yearly" => RecurrenceFreq::Yearly,
                            _ => RecurrenceFreq::Weekly,
                        }),
                    }
                }
                if freq() == RecurrenceFreq::Weekly {
                    div { class: "ax-cal-weekdays", role: "group", "aria-label": "Days of the week",
                        for w in week {
                            {
                                let on = days().contains(&w);
                                let name = weekday_name(w);
                                let letter = name.chars().next().unwrap_or(' ').to_string();
                                rsx! {
                                    button {
                                        key: "{name}",
                                        r#type: "button",
                                        class: "ax-cal-weekday",
                                        class: if on { "ax-checked" },
                                        "aria-pressed": if on { "true" } else { "false" },
                                        "aria-label": "{name}",
                                        title: "{name}",
                                        onclick: move |_| {
                                            let mut d = days();
                                            if on {
                                                // At least one day stays selected.
                                                if d.len() > 1 {
                                                    d.retain(|x| *x != w);
                                                }
                                            } else {
                                                d.push(w);
                                            }
                                            days.set(d);
                                        },
                                        "{letter}"
                                    }
                                }
                            }
                        }
                    }
                }
                if matches!(freq(), RecurrenceFreq::Monthly | RecurrenceFreq::Yearly) {
                    div { class: "ax-col", role: "radiogroup", style: "gap:4px",
                        RadioRow {
                            checked: month_mode() == MonthMode::Day,
                            label: if freq() == RecurrenceFreq::Yearly { format!("On {month_name} {}", start.day()) } else { format!("On day {}", start.day()) },
                            onselect: move |_| month_mode.set(MonthMode::Day),
                        }
                        if n <= 4 {
                            RadioRow {
                                checked: month_mode() == MonthMode::Nth,
                                label: if freq() == RecurrenceFreq::Yearly { format!("On the {} {wd_name} of {month_name}", ordinal_name(n)) } else { format!("On the {} {wd_name}", ordinal_name(n)) },
                                onselect: move |_| month_mode.set(MonthMode::Nth),
                            }
                        }
                        if is_last {
                            RadioRow {
                                checked: month_mode() == MonthMode::Last,
                                label: if freq() == RecurrenceFreq::Yearly { format!("On the last {wd_name} of {month_name}") } else { format!("On the last {wd_name}") },
                                onselect: move |_| month_mode.set(MonthMode::Last),
                            }
                        }
                        if long_month && month_mode() == MonthMode::Day {
                            div { class: "ax-muted ax-small-text", {format!("Some months have fewer than {} days. The event will occur on the last day of the month.", start.day())} }
                        }
                    }
                }
                div { class: "ax-cal-repeat-row ax-cal-repeat-end",
                    span { class: "ax-cal-repeat-label", "End" }
                    div { class: "ax-col", role: "radiogroup", "aria-label": "End", style: "gap:6px",
                        RadioRow { checked: end_mode() == EndMode::Never, label: "No end date", onselect: move |_| end_mode.set(EndMode::Never) }
                        div { class: "ax-row", style: "gap:8px",
                            RadioRow { checked: end_mode() == EndMode::Until, label: "End by", onselect: move |_| end_mode.set(EndMode::Until) }
                            DateField {
                                value: until(),
                                label: "End by",
                                week_start,
                                disabled: end_mode() != EndMode::Until,
                                onchange: move |d| until.set(d),
                            }
                        }
                        div { class: "ax-row", style: "gap:8px",
                            RadioRow { checked: end_mode() == EndMode::Count, label: "End after", onselect: move |_| end_mode.set(EndMode::Count) }
                            input {
                                class: "ax-input ax-cal-repeat-n",
                                r#type: "number",
                                min: "1",
                                max: "999",
                                "aria-label": "Number of occurrences",
                                disabled: end_mode() != EndMode::Count,
                                value: "{count}",
                                oninput: move |e| if let Ok(v) = e.value().parse::<u32>() { count.set(v.clamp(1, 999)) },
                                onkeydown: move |e| keys::stop_typing_keys(&e),
                            }
                            span { "occurrences" }
                        }
                    }
                }
                div { class: "ax-cal-repeat-summary", "{summary}" }
            }
        }
    }
}

#[component]
fn RadioRow(checked: bool, #[props(into)] label: String, onselect: EventHandler<()>) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: "ax-cal-radio",
            class: if checked { "ax-checked" },
            role: "radio",
            "aria-checked": if checked { "true" } else { "false" },
            onclick: move |_| onselect.call(()),
            span { class: "ax-cal-radio-dot" }
            span { "{label}" }
        }
    }
}
