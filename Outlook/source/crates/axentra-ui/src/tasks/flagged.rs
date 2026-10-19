//! "Flagged email" (3.12): flagged messages shown as tasks. Completing one
//! marks its flag complete; clicking it opens the message in Mail.

use std::collections::HashMap;

use axentra_core::*;
use dioxus::prelude::*;

use super::actions;
use super::ctx::{View, use_todo};
use super::dates::{self, DueState};
use super::task_row::{CHECK_SVG, CategoryChip};
use crate::kit::{Button, EmptyState, Icon, Menu, MenuItem, Variant};
use crate::state::{AppState, NavRequest, store, use_app};

fn set_flag(app: AppState, id: MessageId, status: FlagStatus, due: Option<chrono::NaiveDate>) {
    if app.check(store().set_flag(&[id], status, due)).is_some() {
        app.bump_mail();
        app.bump_tasks();
    }
}

#[component]
pub fn FlaggedPane() -> Element {
    let app = use_app();
    let ctx = use_todo();
    let _ = app.mail_rev();
    let view = View::Smart(SmartList::Flagged);
    let enabled = ctx.prefs.read().flagged_email;
    let completed_open = ctx.prefs.read().view(view).completed_open;
    let mut focused = use_signal(|| None::<MessageId>);
    let s = store();
    let open = if enabled { s.flagged_messages().unwrap_or_default() } else { Vec::new() };
    let done = if enabled { s.completed_flagged_messages().unwrap_or_default() } else { Vec::new() };
    let colors: HashMap<String, String> = s.categories().unwrap_or_default().into_iter().map(|c| (c.name, c.color)).collect();
    let done_count = done.len();
    // Roving focus over the visible rows (keyboard spec 14).
    let mut order: Vec<MessageId> = open.iter().map(|m| m.id).collect();
    if completed_open {
        order.extend(done.iter().map(|m| m.id));
    }
    let tab_stop = focused().filter(|id| order.contains(id)).or(order.first().copied());
    let keys_order = order.clone();
    // Completing a row moves it out of place: focus its neighbour (spec 3.8).
    let advance = use_callback(move |id: MessageId| {
        let Some(i) = order.iter().position(|x| *x == id) else { return };
        if let Some(&n) = order.get(i + 1).or_else(|| i.checked_sub(1).and_then(|j| order.get(j))) {
            focused.set(Some(n));
            actions::focus_later(&format!("[data-flag-row=\"{n}\"] .ax-todo-check"), 80);
        }
    });
    rsx! {
        section { class: "ax-pane ax-todo-list", "aria-label": "Flagged email",
            header { class: "ax-todo-head",
                div { class: "ax-todo-head-row",
                    h2 { class: "ax-todo-title", "Flagged email" }
                }
            }
            div { class: "ax-todo-scroll",
                div { class: "ax-todo-content",
                    if !enabled {
                        EmptyState { icon: "flag", title: "Flagged email is turned off",
                            Button { label: "Turn on", variant: Variant::Primary, onclick: move |_| ctx.update_prefs(|p| p.flagged_email = true) }
                        }
                    } else if open.is_empty() && done.is_empty() {
                        EmptyState { icon: "flag", title: "No flagged email", subtitle: "Flag a message in Mail and it shows up here as a task." }
                    } else {
                        div {
                            class: "ax-todo-grid",
                            role: "grid",
                            "aria-label": "Flagged email",
                            onkeydown: move |e: KeyboardEvent| {
                                let pos = focused.peek().and_then(|f| keys_order.iter().position(|x| *x == f)).unwrap_or(0);
                                let next = match e.key() {
                                    Key::ArrowDown => (pos + 1).min(keys_order.len().saturating_sub(1)),
                                    Key::ArrowUp => pos.saturating_sub(1),
                                    Key::Home => 0,
                                    Key::End => keys_order.len().saturating_sub(1),
                                    _ => return,
                                };
                                e.prevent_default();
                                if let Some(&id) = keys_order.get(next) {
                                    focused.set(Some(id));
                                    actions::focus(&format!("[data-flag-row=\"{id}\"]"));
                                }
                            },
                            for m in open {
                                FlaggedRow {
                                    key: "{m.id}",
                                    cats: m.categories.iter().map(|c| (c.clone(), colors.get(c).cloned().unwrap_or_else(|| "gray".into()))).collect(),
                                    tab_stop: tab_stop == Some(m.id),
                                    on_focus: move |id| focused.set(Some(id)),
                                    on_completed: advance,
                                    message: m,
                                }
                            }
                            if done_count > 0 {
                                button {
                                    r#type: "button",
                                    class: "ax-todo-completed-head",
                                    "aria-expanded": if completed_open { "true" } else { "false" },
                                    onclick: move |_| ctx.update_view(view, |v| v.completed_open = !v.completed_open),
                                    Icon { name: "chevron_down", size: 12, class: if completed_open { "ax-todo-chevron" } else { "ax-todo-chevron ax-collapsed" } }
                                    span { "Completed" }
                                    span { class: "ax-todo-completed-count", "{done_count}" }
                                }
                                if completed_open {
                                    for m in done {
                                        FlaggedRow {
                                            key: "{m.id}",
                                            cats: Vec::new(),
                                            tab_stop: tab_stop == Some(m.id),
                                            on_focus: move |id| focused.set(Some(id)),
                                            on_completed: advance,
                                            message: m,
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
}

#[component]
fn FlaggedRow(
    message: Message,
    cats: Vec<(String, String)>,
    tab_stop: bool,
    on_focus: EventHandler<MessageId>,
    /// Completed or reopened from the keyboard or the circle.
    on_completed: Callback<MessageId>,
) -> Element {
    let app = use_app();
    let ctx = use_todo();
    let mut menu = use_signal(|| None::<(f64, f64)>);
    let id = message.id;
    let done = message.flag == FlagStatus::Complete;
    let important = message.importance == Importance::High;
    let today = (ctx.today)();
    let due = message.flag_due.map(|d| (dates::due_row_label(d, today), dates::due_state(d, today)));
    let check_label = if done { "Mark as not completed" } else { "Mark as completed" };
    let star_label = if important { "Remove importance" } else { "Mark as important" };
    let msg = message.clone();
    let due_now = message.flag_due;
    let items = vec![
        MenuItem::new("open", "Open email").icon("open"),
        if done { MenuItem::new("undone", "Mark as not completed").icon("circle") } else { MenuItem::new("done", "Mark as completed").icon("checkmark_circle") },
        MenuItem::new("clear", "Clear flag").icon("flag_off"),
        MenuItem::divider(),
        MenuItem::new("due:today", "Due today").icon("calendar_today"),
        MenuItem::new("due:tomorrow", "Due tomorrow").icon("calendar_arrow_right"),
        MenuItem::new("due:none", "Remove due date").icon("calendar_cancel").disabled(message.flag_due.is_none()),
    ];
    rsx! {
        div {
            class: "ax-todo-card",
            class: if done { "ax-todo-done" },
            role: "row",
            "data-ctx": "1",
            "data-flag-row": "{id}",
            tabindex: if tab_stop { "0" } else { "-1" },
            title: "Open email",
            onfocus: move |_| on_focus.call(id),
            onclick: move |_| app.navigate(NavRequest::Message(id)),
            onkeydown: move |e: KeyboardEvent| {
                // Enter opens the email; Ctrl+D / Insert completes or reopens it.
                match crate::kit::keys::chord(&e).as_str() {
                    "Enter" => app.navigate(NavRequest::Message(id)),
                    "Ctrl+D" | "Insert" => {
                        if !done {
                            actions::chime(ctx);
                            ctx.say("Task completed");
                        }
                        set_flag(app, id, if done { FlagStatus::Flagged } else { FlagStatus::Complete }, due_now);
                        on_completed.call(id);
                    }
                    _ => return,
                }
                e.prevent_default();
                e.stop_propagation();
            },
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                let c = e.client_coordinates();
                menu.set(Some((c.x, c.y)));
            },
            button {
                r#type: "button",
                class: "ax-todo-check",
                class: if done { "ax-checked" },
                role: "checkbox",
                "aria-checked": if done { "true" } else { "false" },
                "aria-label": "{check_label}",
                title: "{check_label}",
                tabindex: if tab_stop { "0" } else { "-1" },
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    if !done {
                        actions::chime(ctx);
                        ctx.say("Task completed");
                    }
                    set_flag(app, id, if done { FlagStatus::Flagged } else { FlagStatus::Complete }, msg.flag_due);
                    on_completed.call(id);
                },
                onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter || e.key() == Key::Character(" ".into()) { e.stop_propagation() },
                span { class: "ax-todo-check-svg", dangerous_inner_html: CHECK_SVG }
            }
            div { class: "ax-todo-card-main", role: "gridcell",
                div { class: "ax-todo-card-title",
                    span { class: "ax-todo-title-icon", Icon { name: "mail", size: 12 } }
                    span { if message.subject.is_empty() { "(No subject)" } else { "{message.subject}" } }
                }
                div { class: "ax-todo-card-sub",
                    span { class: "ax-todo-sub-item", "{message.from.display()}" }
                    if let Some((label, state)) = &due {
                        span {
                            class: "ax-todo-sub-item",
                            class: if *state == DueState::Overdue && !done { "ax-todo-overdue" },
                            class: if *state == DueState::Today && !done { "ax-todo-due-today" },
                            Icon { name: "calendar_ltr", size: 12 }
                            "{label}"
                        }
                    }
                    if !cats.is_empty() {
                        span { class: "ax-todo-sub-item ax-todo-cats",
                            for (name, color) in cats.iter().take(2) {
                                CategoryChip { key: "{name}", name: name.clone(), color: color.clone() }
                            }
                        }
                    }
                }
            }
            button {
                r#type: "button",
                class: "ax-todo-star",
                class: if important { "ax-toggled" },
                "aria-pressed": if important { "true" } else { "false" },
                "aria-label": "{star_label}",
                title: "{star_label}",
                tabindex: if tab_stop { "0" } else { "-1" },
                onclick: {
                    let m = message.clone();
                    move |e: MouseEvent| {
                        e.stop_propagation();
                        let mut m = m.clone();
                        m.importance = if important { Importance::Normal } else { Importance::High };
                        if app.check(store().update_message(&m)).is_some() {
                            app.bump_mail();
                        }
                    }
                },
                Icon { name: if important { "star_filled" } else { "star" } }
            }
            if let Some((x, y)) = menu() {
                Menu {
                    items: items.clone(),
                    x,
                    y,
                    on_select: {
                        let due_now = message.flag_due;
                        move |cmd: String| match cmd.as_str() {
                            "open" => app.navigate(NavRequest::Message(id)),
                            "done" => set_flag(app, id, FlagStatus::Complete, due_now),
                            "undone" => set_flag(app, id, FlagStatus::Flagged, due_now),
                            "clear" => {
                                set_flag(app, id, FlagStatus::NotFlagged, None);
                                app.toast("Flag cleared");
                            }
                            "due:today" => set_flag(app, id, FlagStatus::Flagged, Some(today)),
                            "due:tomorrow" => set_flag(app, id, FlagStatus::Flagged, Some(today + chrono::Duration::days(1))),
                            "due:none" => set_flag(app, id, FlagStatus::Flagged, None),
                            _ => {}
                        }
                    },
                    on_close: move |_| menu.set(None),
                }
            }
        }
    }
}
