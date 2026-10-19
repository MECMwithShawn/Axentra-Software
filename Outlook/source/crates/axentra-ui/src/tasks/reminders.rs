//! Task reminders (3.10.2). A check every few seconds fires the reminders
//! that came due since the previous check as cards at the bottom right,
//! titled with the task, with "Complete" and "Snooze 10 minutes".
//!
//! Self-contained (no To Do context needed) so it can run for the whole
//! app. It may be mounted more than once (the To Do module mounts one, the
//! shell may mount another): only one instance per window fires reminders.
//! Honours "Turn on reminder notifications" in To Do settings.

use axentra_core::*;
use chrono::{Duration, Utc};
use dioxus::core::{current_scope_id, provide_root_context};
use dioxus::prelude::*;

use super::ctx::{Prefs, TodoCtx, View};
use super::dates;
use crate::kit::{Button, Icon, IconButton, Variant};
use crate::state::{NavRequest, store, use_app};

/// How often due reminders are looked for.
const CHECK_SECS: u64 = 10;
/// "Snooze 10 minutes".
const SNOOZE_MINUTES: i64 = 10;

/// The mounted [`TaskReminders`] that fires reminders, kept in the root
/// scope so every instance in the window sees it.
#[derive(Clone, Copy)]
struct Runner(Signal<Option<ScopeId>>);

fn runner() -> Runner {
    try_consume_context::<Runner>().unwrap_or_else(|| provide_root_context(Runner(Signal::new_in_scope(None, ScopeId::ROOT))))
}

#[component]
pub fn TaskReminders() -> Element {
    let app = use_app();
    let _ = app.tasks_rev();
    let me = use_hook(current_scope_id);
    let Runner(mut owner) = use_hook(runner);
    // The first instance to mount runs; another takes over when it goes.
    use_effect(move || {
        if owner().is_none() {
            owner.set(Some(me));
        }
    });
    use_drop(move || {
        if *owner.peek() == Some(me) {
            owner.set(None);
        }
    });
    // Tasks whose reminder fired and hasn't been dealt with yet.
    let mut fired = use_signal(Vec::<TaskId>::new);
    use_future(move || async move {
        let mut last = Utc::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(CHECK_SECS)).await;
            let now = Utc::now();
            if *owner.peek() == Some(me)
                && Prefs::load().reminders
                && let Ok(due) = store().task_reminders_between(last, now)
                && !due.is_empty()
            {
                let mut f = fired.write();
                for t in due {
                    if !f.contains(&t.id) {
                        f.push(t.id);
                    }
                }
            }
            last = now;
        }
    });
    if owner() != Some(me) {
        return rsx! {};
    }
    let tasks: Vec<Task> = fired.read().iter().filter_map(|id| store().task(*id).ok().flatten()).filter(|t| !t.is_completed).collect();
    if tasks.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "ax-todo-reminders", role: "region", "aria-label": "Reminders", "aria-live": "polite",
            for t in tasks {
                ReminderCard { key: "{t.id}", task: t, on_close: move |id: TaskId| fired.write().retain(|x| *x != id) }
            }
        }
    }
}

#[component]
fn ReminderCard(task: Task, on_close: EventHandler<TaskId>) -> Element {
    let app = use_app();
    let todo = try_use_context::<TodoCtx>();
    let id = task.id;
    let list_id = task.list_id;
    let today = dates::today();
    let h24 = app.settings.read().use_24h_time;
    let when = task.reminder.map(|r| dates::reminder_lines(r, today, h24).0).unwrap_or_default();
    let open = move |_| {
        on_close.call(id);
        match todo {
            Some(ctx) => {
                ctx.go(View::List(list_id));
                ctx.open(id);
            }
            None => app.navigate(NavRequest::TaskList(list_id)),
        }
    };
    rsx! {
        div { class: "ax-todo-reminder", role: "alertdialog", "aria-label": "Reminder: {task.title}",
            div { class: "ax-todo-reminder-head",
                Icon { name: "alert" }
                span { class: "ax-grow", "Reminder" }
                IconButton { icon: "dismiss", title: "Dismiss", small: true, onclick: move |_| on_close.call(id) }
            }
            button { r#type: "button", class: "ax-todo-reminder-title", title: "Open task", onclick: open, "{task.title}" }
            div { class: "ax-todo-reminder-when", "{when}" }
            div { class: "ax-todo-reminder-actions",
                Button {
                    label: "Complete",
                    variant: Variant::Primary,
                    small: true,
                    onclick: move |_| {
                        on_close.call(id);
                        if app.check(store().set_task_completed(id, true)).is_some() {
                            app.bump_tasks();
                        }
                    },
                }
                Button {
                    label: "Snooze 10 minutes",
                    small: true,
                    onclick: move |_| {
                        on_close.call(id);
                        if app.check(store().snooze_task_reminder(id, Utc::now() + Duration::minutes(SNOOZE_MINUTES))).is_some() {
                            app.bump_tasks();
                        }
                    },
                }
            }
        }
    }
}
