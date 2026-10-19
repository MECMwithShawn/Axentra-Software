//! Event reminders: a timer asks the store which reminders are due
//! (persisted snooze / dismiss state) and shows them in a non-modal reminder
//! window with Join, Snooze and Dismiss (Alt+S / Alt+D / Alt+O on the first).
//!
//! `ReminderHost` is safe to mount more than once (the module mounts one;
//! the shell can mount one so reminders fire in every module): only the
//! oldest mounted host is active.

use std::sync::Mutex;

use axentra_core::*;
use chrono::{DateTime, Duration, Local, Utc};
use dioxus::prelude::*;

use super::actions::{join_url, open_external};
use super::colors::ramp;
use super::event_block::display_title;
use crate::fmt;
use crate::kit::{Button, Icon, IconButton, MenuButton, MenuItem, Variant, keys};
use crate::state::{NavRequest, store, use_app};

static HOSTS: Mutex<Vec<u64>> = Mutex::new(Vec::new());
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn register() -> u64 {
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    HOSTS.lock().unwrap_or_else(|p| p.into_inner()).push(id);
    id
}

fn unregister(id: u64) {
    HOSTS.lock().unwrap_or_else(|p| p.into_inner()).retain(|h| *h != id);
}

fn is_active(id: u64) -> bool {
    HOSTS.lock().unwrap_or_else(|p| p.into_inner()).first() == Some(&id)
}

/// "Starts in 5 min", "Now", "10 min overdue", "Tomorrow at 9:00 AM".
pub fn status_text(start: DateTime<Utc>, now: DateTime<Utc>, h24: bool) -> String {
    let mins = (start - now).num_seconds().div_euclid(60) + i64::from((start - now).num_seconds().rem_euclid(60) > 0);
    let today = now.with_timezone(&Local).date_naive();
    let day = local_date(start);
    if mins <= 0 {
        let late = -mins;
        return match late {
            0 => "Now".into(),
            l if l < 60 => format!("{l} min overdue"),
            l if l < 24 * 60 => format!("{} hr overdue", l / 60),
            l => format!("{} days overdue", l / (24 * 60)),
        };
    }
    if day == today {
        if mins < 60 { format!("Starts in {mins} min") } else { format!("Today at {}", fmt::time(start, h24)) }
    } else if day == today + Duration::days(1) {
        format!("Tomorrow at {}", fmt::time(start, h24))
    } else {
        format!("{} at {}", day.format("%a %-m/%-d"), fmt::time(start, h24))
    }
}

/// Snooze choices for a reminder: (id, label, until).
fn snooze_choices(start: DateTime<Utc>, now: DateTime<Utc>) -> Vec<(String, String, DateTime<Utc>)> {
    let mut out = Vec::new();
    if start - Duration::minutes(5) > now {
        out.push(("before".into(), "5 minutes before start".into(), start - Duration::minutes(5)));
    }
    if start > now {
        out.push(("start".into(), "At start time".into(), start));
    }
    for (m, l) in [(5, "5 minutes"), (10, "10 minutes"), (15, "15 minutes"), (30, "30 minutes"), (60, "1 hour"), (120, "2 hours")] {
        out.push((format!("m{m}"), l.into(), now + Duration::minutes(m)));
    }
    out
}

#[component]
pub fn ReminderHost() -> Element {
    let app = use_app();
    let id = use_hook(register);
    use_drop(move || unregister(id));
    let mut due = use_signal(Vec::<(Occurrence, DateTime<Utc>)>::new);
    // Hidden with the close button until the set of due reminders changes.
    let mut hidden_for = use_signal(Vec::<(EventId, i64)>::new);

    let mut refresh = move || {
        if !is_active(id) {
            return;
        }
        let list = store().due_reminders(Utc::now()).unwrap_or_default();
        if list != *due.peek() {
            due.set(list);
        }
    };
    use_future(move || async move {
        loop {
            refresh();
            let now = Utc::now();
            let next = store().next_reminder_at(now).ok().flatten();
            let wait = next.map(|n| (n - now).num_milliseconds().clamp(1_000, 30_000)).unwrap_or(30_000);
            tokio::time::sleep(std::time::Duration::from_millis(wait as u64 + 50)).await;
        }
    });
    // Re-check straight after calendar edits (new reminder, deleted event…).
    use_effect(move || {
        let _ = app.calendar_rev();
        refresh();
    });

    let list = due();
    let keys_now: Vec<(EventId, i64)> = list.iter().map(|(o, _)| (o.event.id, o.start.timestamp())).collect();
    if list.is_empty() || hidden_for() == keys_now || !is_active(id) {
        return rsx! {};
    }
    let h24 = app.settings.read().use_24h_time;
    let now = Utc::now();
    let n = list.len();
    let title = if n == 1 { "1 reminder".to_string() } else { format!("{n} reminders") };
    // Overdue first (oldest first), then upcoming by start.
    let mut list = list;
    list.sort_by_key(|(o, _)| o.start);
    let first = list.first().map(|(o, _)| o.clone());
    let dismiss_all = list.iter().map(|(o, _)| (o.event.id, o.start)).collect::<Vec<_>>();
    let keys_close = keys_now.clone();
    rsx! {
        div {
            class: "ax-cal-reminders",
            role: "dialog",
            "aria-modal": "false",
            "aria-label": "{title}",
            onmousedown: move |e| e.stop_propagation(),
            onkeydown: move |e: KeyboardEvent| {
                let Some(o) = first.clone() else { return };
                match keys::chord(&e).as_str() {
                    "Alt+D" => {
                        e.prevent_default();
                        dismiss(app, o.event.id, o.start);
                    }
                    "Alt+S" => {
                        e.prevent_default();
                        snooze(app, o.event.id, o.start, Utc::now() + Duration::minutes(5));
                    }
                    "Alt+O" => {
                        e.prevent_default();
                        app.navigate(NavRequest::Event(o.event.id));
                    }
                    "Escape" => {
                        e.stop_propagation();
                        hidden_for.set(keys_now.clone());
                    }
                    _ => {}
                }
            },
            div { class: "ax-cal-reminders-head",
                Icon { name: "alert" }
                span { class: "ax-grow ax-semibold", "{title}" }
                IconButton { icon: "dismiss", title: "Close (reminders come back when something new is due)", onclick: move |_| hidden_for.set(keys_close.clone()) }
            }
            div { class: "ax-cal-reminders-list",
                for (o, _) in list {
                    ReminderRow { key: "{o.event.id}-{o.start.timestamp()}", status: status_text(o.start, now, h24), occ: o, now }
                }
            }
            div { class: "ax-cal-reminders-foot",
                Button {
                    label: "Dismiss all",
                    onclick: move |_| {
                        for (id, start) in &dismiss_all {
                            let _ = store().dismiss_reminder(*id, *start);
                        }
                        app.bump_calendar();
                    },
                }
            }
        }
    }
}

fn dismiss(app: crate::state::AppState, id: EventId, start: DateTime<Utc>) {
    if app.check(store().dismiss_reminder(id, start)).is_some() {
        app.bump_calendar();
    }
}

fn snooze(app: crate::state::AppState, id: EventId, start: DateTime<Utc>, until: DateTime<Utc>) {
    if app.check(store().snooze_reminder(id, start, until)).is_some() {
        app.bump_calendar();
    }
}

#[component]
fn ReminderRow(occ: Occurrence, status: String, now: DateTime<Utc>) -> Element {
    let app = use_app();
    let e = occ.event.clone();
    let color = ramp(&occ.color).primary;
    let choices = snooze_choices(occ.start, now);
    let items: Vec<MenuItem> = choices.iter().map(|(id, label, _)| MenuItem::new(id.clone(), label.clone())).collect();
    let (id, start) = (e.id, occ.start);
    let join = join_url(&e);
    let overdue = occ.start <= now;
    rsx! {
        div { class: "ax-cal-reminder", role: "group", "aria-label": display_title(&e),
            span { class: "ax-cal-reminder-bar", style: "background:{color}" }
            div { class: "ax-col ax-grow",
                button {
                    r#type: "button",
                    class: "ax-cal-reminder-title",
                    title: "Open in Calendar",
                    onclick: move |_| app.navigate(NavRequest::Event(id)),
                    {display_title(&e)}
                }
                span { class: "ax-cal-reminder-status", class: if overdue { "ax-overdue" }, "{status}" }
                if !e.location.trim().is_empty() { span { class: "ax-muted ax-small-text ax-ellipsis", "{e.location}" } }
                div { class: "ax-row", style: "gap:6px;margin-top:6px",
                    if let Some(url) = join {
                        Button { label: "Join", icon: "video", variant: Variant::Primary, small: true, onclick: move |_| open_external(&url) }
                    }
                    Button { label: "Snooze", icon: "snooze", small: true, title: "Snooze 5 minutes (Alt+S)", onclick: move |_| snooze(app, id, start, Utc::now() + Duration::minutes(5)) }
                    MenuButton {
                        label: "Snooze for",
                        icon: "chevron_down",
                        icon_only: true,
                        class: "ax-small",
                        items,
                        on_select: move |which: String| {
                            if let Some((_, _, until)) = choices.iter().find(|(c, _, _)| *c == which) {
                                snooze(app, id, start, *until);
                            }
                        },
                    }
                    Button { label: "Dismiss", small: true, title: "Dismiss (Alt+D)", onclick: move |_| dismiss(app, id, start) }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};

    #[test]
    fn reminder_status_texts() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let t = |d: NaiveDate, h, m| local_to_utc(d, NaiveTime::from_hms_opt(h, m, 0).unwrap());
        let now = t(day, 8, 55);
        assert_eq!(status_text(t(day, 9, 0), now, false), "Starts in 5 min");
        assert_eq!(status_text(t(day, 8, 55), now, false), "Now");
        assert_eq!(status_text(t(day, 8, 45), now, false), "10 min overdue");
        assert_eq!(status_text(t(day, 11, 0), now, false), "Today at 11:00 AM");
        assert_eq!(status_text(t(day + Duration::days(1), 9, 0), now, false), "Tomorrow at 9:00 AM");
        let ids: Vec<String> = snooze_choices(t(day, 9, 0), now).into_iter().map(|c| c.0).collect();
        assert_eq!(ids[..2], ["start".to_string(), "m5".to_string()]);
        assert_eq!(snooze_choices(t(day, 10, 0), now)[0].1, "5 minutes before start");
    }
}
