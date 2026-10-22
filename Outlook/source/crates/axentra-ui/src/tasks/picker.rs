//! "Pick a date" / "Pick a date & time" popover and the custom repeat
//! dialog (3.10). Both save explicitly with "Save" / "Cancel".

use axentra_core::*;
use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};
use dioxus::prelude::*;

use super::actions;
use super::ctx::{Picker, PickerKind, Target, use_todo};
use super::dates;
use crate::kit::{Button, Dialog, Icon, Variant, keys};
use crate::state::{store, use_app};

/// Renders whichever picker is open.
#[component]
pub fn PickerHost() -> Element {
    let ctx = use_todo();
    let Some(p) = (ctx.picker)() else { return rsx! {} };
    rsx! {
        match p.kind {
            PickerKind::CustomRepeat => rsx! { RepeatDialog { picker: p } },
            _ => rsx! { DatePopover { picker: p } },
        }
    }
}

/// The first target task, if the picker edits tasks.
fn first_task(target: &Target) -> Option<Task> {
    match target {
        Target::Tasks(ids) => ids.first().and_then(|id| store().task(*id).ok().flatten()),
        Target::NewTask => None,
    }
}

#[component]
fn DatePopover(picker: Picker) -> Element {
    let app = use_app();
    let mut ctx = use_todo();
    let today = (ctx.today)();
    let h24 = app.settings.read().use_24h_time;
    let week_start = app.settings.read().week_start;
    let with_time = picker.kind == PickerKind::Reminder;
    let initial = use_hook(|| {
        let task = first_task(&picker.target);
        let draft = ctx.draft.peek().clone();
        if with_time {
            let r = task.as_ref().and_then(|t| t.reminder).or(draft.reminder);
            r.map(|r| (local_date(r), crate::fmt::time(r, h24)))
                .unwrap_or((today + Duration::days(1), dates::time_label(chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap_or_default(), h24)))
        } else {
            (task.and_then(|t| t.due).or(draft.due).unwrap_or(today), String::new())
        }
    });
    let mut chosen = use_signal(|| initial.0);
    let mut month = use_signal(|| initial.0.with_day(1).unwrap_or(initial.0));
    let mut time = use_signal(|| initial.1.clone());
    let target = picker.target.clone();
    let mut save = move |_: ()| {
        let d = chosen();
        match (&target, with_time) {
            (Target::Tasks(ids), false) => actions::set_due(app, ids, Some(d)),
            (Target::NewTask, false) => ctx.draft.write().due = Some(d),
            (_, true) => {
                let Some(t) = dates::parse_time(&time()) else { return };
                let at = Some(local_to_utc(d, t));
                match &target {
                    Target::Tasks(ids) => actions::set_reminder(app, ids, at),
                    Target::NewTask => ctx.draft.write().reminder = at,
                }
            }
        }
        ctx.picker.set(None);
    };
    let time_ok = !with_time || dates::parse_time(&time()).is_some();
    let mut save_key = save.clone();
    let mut save_time = save.clone();
    let title = if with_time { "Pick a date & time" } else { "Pick a date" };
    let first = month();
    let lead = (first.weekday().num_days_from_monday() + 7 - week_start.num_days_from_monday()) % 7;
    let grid_start = first - Duration::days(lead as i64);
    let headers: Vec<String> = (0..7).map(|i| weekday_letter(grid_start + Duration::days(i))).collect();
    let days: Vec<NaiveDate> = (0..42).map(|i| grid_start + Duration::days(i)).collect();
    let (x, y) = (picker.x, picker.y);
    rsx! {
        div { class: "ax-menu-backdrop", onclick: move |_| ctx.picker.set(None) }
        div {
            class: "ax-todo-picker",
            role: "dialog",
            "aria-label": "{title}",
            tabindex: "-1",
            style: "left:{x}px;top:{y}px;transform:translate(min(0px, calc(100vw - {x}px - 100% - 8px)), min(0px, calc(100vh - {y}px - 100% - 8px)))",
            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
            onclick: move |e| e.stop_propagation(),
            onkeydown: move |e: KeyboardEvent| {
                let step = match e.key() {
                    Key::ArrowLeft => -1,
                    Key::ArrowRight => 1,
                    Key::ArrowUp => -7,
                    Key::ArrowDown => 7,
                    Key::Escape => {
                        e.stop_propagation();
                        ctx.picker.set(None);
                        return;
                    }
                    Key::Enter => {
                        e.prevent_default();
                        save_key(());
                        return;
                    }
                    _ => return,
                };
                e.prevent_default();
                e.stop_propagation();
                let d = chosen() + Duration::days(step);
                chosen.set(d);
                month.set(d.with_day(1).unwrap_or(d));
            },
            div { class: "ax-todo-picker-head",
                span { class: "ax-grow ax-semibold", "{first.format(\"%B %Y\")}" }
                button {
                    r#type: "button",
                    class: "ax-icon-btn ax-small",
                    title: "Previous month",
                    "aria-label": "Previous month",
                    onclick: move |_| month.set(first.checked_sub_months(Months::new(1)).unwrap_or(first)),
                    Icon { name: "arrow_up", size: 16 }
                }
                button {
                    r#type: "button",
                    class: "ax-icon-btn ax-small",
                    title: "Next month",
                    "aria-label": "Next month",
                    onclick: move |_| month.set(first.checked_add_months(Months::new(1)).unwrap_or(first)),
                    Icon { name: "arrow_down", size: 16 }
                }
            }
            div { class: "ax-todo-cal", role: "grid", "aria-label": "{first.format(\"%B %Y\")}",
                for (i, h) in headers.iter().enumerate() {
                    span { key: "h{i}", class: "ax-todo-cal-wd", role: "columnheader", "{h}" }
                }
                for d in days {
                    button {
                        key: "{d}",
                        r#type: "button",
                        role: "gridcell",
                        class: "ax-todo-cal-day",
                        class: if d.month() != first.month() { "ax-todo-cal-out" },
                        class: if d == today { "ax-todo-cal-today" },
                        class: if d == chosen() { "ax-selected" },
                        "aria-selected": if d == chosen() { "true" } else { "false" },
                        "aria-current": if d == today { "date" } else { "false" },
                        "aria-label": "{d.format(\"%A, %B %-d, %Y\")}",
                        tabindex: "-1",
                        onclick: move |_| chosen.set(d),
                        "{d.day()}"
                    }
                }
            }
            if with_time {
                label { class: "ax-todo-picker-time",
                    Icon { name: "clock", size: 16 }
                    // Combobox: 30-minute suggestions, any typed time accepted.
                    input {
                        class: "ax-input ax-grow",
                        r#type: "text",
                        role: "combobox",
                        "aria-label": "Time",
                        "aria-invalid": if time_ok { "false" } else { "true" },
                        list: "ax-todo-times",
                        autocomplete: "off",
                        value: "{time}",
                        oninput: move |e| time.set(e.value()),
                        onkeydown: move |e: KeyboardEvent| {
                            keys::stop_typing_keys(&e);
                            if e.key() == Key::Enter {
                                e.prevent_default();
                                save_time(());
                            }
                        },
                    }
                    datalist { id: "ax-todo-times",
                        for (v, l) in dates::time_options(h24) {
                            option { key: "{v}", value: "{l}" }
                        }
                    }
                }
                if !time_ok {
                    div { class: "ax-todo-field-error", role: "alert", "Enter a time like 9:30 AM." }
                }
            }
            div { class: "ax-todo-picker-actions",
                Button { label: "Save", variant: Variant::Primary, small: true, disabled: !time_ok, onclick: move |_| save(()) }
                Button { label: "Cancel", small: true, onclick: move |_| ctx.picker.set(None) }
            }
        }
    }
}

fn weekday_letter(d: NaiveDate) -> String {
    d.format("%a").to_string().chars().take(2).collect()
}

const UNITS: [(&str, &str); 4] = [("days", "days"), ("weeks", "weeks"), ("months", "months"), ("years", "years")];

#[component]
fn RepeatDialog(picker: Picker) -> Element {
    let app = use_app();
    let mut ctx = use_todo();
    let week_start = app.settings.read().week_start;
    let today = (ctx.today)();
    let existing = use_hook(|| {
        let task = first_task(&picker.target);
        let base = task.as_ref().and_then(|t| t.due).unwrap_or(today);
        let rule = task.and_then(|t| t.recurrence).or(ctx.draft.peek().repeat.clone());
        (rule, base)
    });
    let (rule, base) = existing.clone();
    let mut every = use_signal(|| rule.as_ref().map(|r| r.interval.max(1)).unwrap_or(1).to_string());
    let mut unit = use_signal(|| {
        match rule.as_ref().map(|r| r.freq) {
            Some(RecurrenceFreq::Daily) => "days",
            Some(RecurrenceFreq::Monthly) => "months",
            Some(RecurrenceFreq::Yearly) => "years",
            _ => "weeks",
        }
        .to_string()
    });
    let mut days = use_signal(|| rule.as_ref().map(|r| r.by_weekday.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| vec![base.weekday()]));
    let interval: Option<u32> = every().trim().parse().ok().filter(|n| (1..=999).contains(n));
    let target = picker.target.clone();
    let save = move |_| {
        let Some(n) = interval else { return };
        let freq = match unit().as_str() {
            "days" => RecurrenceFreq::Daily,
            "months" => RecurrenceFreq::Monthly,
            "years" => RecurrenceFreq::Yearly,
            _ => RecurrenceFreq::Weekly,
        };
        let by_weekday = if freq == RecurrenceFreq::Weekly { days() } else { Vec::new() };
        let rule = Recurrence { freq, interval: n, by_weekday, ..Default::default() };
        match &target {
            Target::Tasks(ids) => actions::set_repeat(app, ids, Some(rule), today),
            Target::NewTask => {
                let mut d = ctx.draft.write();
                d.due = d.due.or(Some(today));
                d.repeat = Some(rule);
            }
        }
        ctx.picker.set(None);
    };
    let order: Vec<Weekday> = std::iter::successors(Some(week_start), |d| Some(d.succ())).take(7).collect();
    rsx! {
        Dialog {
            title: "Repeat",
            width: 360,
            on_close: move |_| ctx.picker.set(None),
            actions: rsx! {
                Button { label: "Save", variant: Variant::Primary, disabled: interval.is_none() || (unit() == "weeks" && days().is_empty()), onclick: save }
                Button { label: "Cancel", onclick: move |_| ctx.picker.set(None) }
            },
            div { class: "ax-todo-repeat",
                label { class: "ax-field-label", r#for: "ax-todo-repeat-n", "Repeat every" }
                div { class: "ax-row", style: "gap:8px",
                    input {
                        id: "ax-todo-repeat-n",
                        class: "ax-input",
                        r#type: "number",
                        min: "1",
                        max: "999",
                        style: "width:80px",
                        value: "{every}",
                        oninput: move |e| every.set(e.value()),
                        onkeydown: move |e| keys::stop_typing_keys(&e),
                    }
                    div { class: "ax-select-wrap ax-grow",
                        select {
                            class: "ax-select",
                            "aria-label": "Unit",
                            onchange: move |e| unit.set(e.value()),
                            for (v, l) in UNITS {
                                option { key: "{v}", value: "{v}", selected: unit() == v, "{l}" }
                            }
                        }
                        Icon { name: "chevron_down", size: 12, class: "ax-select-chevron" }
                    }
                }
                if unit() == "weeks" {
                    div { class: "ax-todo-chips", role: "group", "aria-label": "Days of the week",
                        for d in order {
                            button {
                                key: "{d}",
                                r#type: "button",
                                class: "ax-todo-chip",
                                class: if days().contains(&d) { "ax-selected" },
                                "aria-pressed": if days().contains(&d) { "true" } else { "false" },
                                title: "{d}",
                                onclick: move |_| {
                                    let mut v = days();
                                    if let Some(i) = v.iter().position(|x| *x == d) { v.remove(i); } else { v.push(d); }
                                    days.set(v);
                                },
                                "{weekday_letter_of(d)}"
                            }
                        }
                    }
                }
                if interval.is_none() {
                    div { class: "ax-todo-field-error", role: "alert", "Enter a number from 1 to 999." }
                }
            }
        }
    }
}

fn weekday_letter_of(d: Weekday) -> String {
    format!("{d}").chars().take(2).collect()
}
