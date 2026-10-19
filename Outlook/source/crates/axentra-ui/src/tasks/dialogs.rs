//! Confirmation dialogs (delete task / list), To Do settings (3.3.5) and
//! the To Do keyboard shortcuts list.

use axentra_core::*;
use dioxus::prelude::*;

use super::actions;
use super::ctx::{Confirm, Prefs, TodoDialog, View, use_todo};
use crate::kit::{Button, Dialog, Switch, Variant};
use crate::state::{store, use_app};

#[component]
pub fn ConfirmHost() -> Element {
    let app = use_app();
    let mut ctx = use_todo();
    let Some(c) = (ctx.confirm)() else { return rsx! {} };
    match c {
        Confirm::DeleteTasks(ids) => {
            let (title, body) = if let [id] = ids.as_slice() {
                let name = store().task(*id).ok().flatten().map(|t| t.title).unwrap_or_default();
                ("Delete task?".to_string(), format!("\u{201c}{name}\u{201d} will be permanently deleted."))
            } else {
                ("Delete tasks?".to_string(), format!("{} tasks will be permanently deleted.", ids.len()))
            };
            rsx! {
                Dialog {
                    title,
                    width: 400,
                    on_close: move |_| {
                        ctx.confirm.set(None);
                        actions::refocus_selection(ctx);
                    },
                    actions: rsx! {
                        Button {
                            label: "Delete",
                            variant: Variant::Primary,
                            onclick: move |_| {
                                ctx.confirm.set(None);
                                actions::delete_now(app, ctx, &ids, false);
                            },
                        }
                        Button {
                            label: "Cancel",
                            onclick: move |_| {
                                ctx.confirm.set(None);
                                actions::refocus_selection(ctx);
                            },
                        }
                    },
                    p { class: "ax-todo-dialog-text", "{body}" }
                }
            }
        }
        Confirm::DeleteList(id) => {
            let name = store().task_list(id).ok().flatten().map(|l| l.name).unwrap_or_default();
            rsx! {
                Dialog {
                    title: "Delete list?",
                    width: 400,
                    on_close: move |_| ctx.confirm.set(None),
                    actions: rsx! {
                        Button {
                            label: "Delete list",
                            variant: Variant::Primary,
                            onclick: move |_| {
                                ctx.confirm.set(None);
                                if app.check(store().delete_task_list(id)).is_some() {
                                    if (ctx.view)() == View::List(id) {
                                        ctx.go(View::Smart(SmartList::MyDay));
                                    }
                                    ctx.say("List deleted");
                                    app.bump_tasks();
                                }
                            },
                        }
                        Button { label: "Cancel", onclick: move |_| ctx.confirm.set(None) }
                    },
                    p { class: "ax-todo-dialog-text", "\u{201c}{name}\u{201d} and all its tasks will be permanently deleted. This can't be undone." }
                }
            }
        }
    }
}

#[component]
pub fn DialogHost() -> Element {
    let ctx = use_todo();
    match (ctx.dialog)() {
        Some(TodoDialog::Settings) => rsx! { SettingsDialog {} },
        Some(TodoDialog::Shortcuts) => rsx! { ShortcutsDialog {} },
        None => rsx! {},
    }
}

/// A boolean To Do setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pref {
    ConfirmDelete,
    AddOnTop,
    StarredOnTop,
    Sound,
    Reminders,
    Important,
    Planned,
    All,
    Completed,
    Assigned,
    AutoHide,
    DueToday,
    Flagged,
}

impl Pref {
    fn field(self, p: &mut Prefs) -> &mut bool {
        match self {
            Pref::ConfirmDelete => &mut p.confirm_delete,
            Pref::AddOnTop => &mut p.add_on_top,
            Pref::StarredOnTop => &mut p.starred_on_top,
            Pref::Sound => &mut p.completion_sound,
            Pref::Reminders => &mut p.reminders,
            Pref::Important => &mut p.show_important,
            Pref::Planned => &mut p.show_planned,
            Pref::All => &mut p.show_all,
            Pref::Completed => &mut p.show_completed,
            Pref::Assigned => &mut p.show_assigned,
            Pref::AutoHide => &mut p.auto_hide_empty,
            Pref::DueToday => &mut p.due_today_in_my_day,
            Pref::Flagged => &mut p.flagged_email,
        }
    }
}

/// One settings switch bound to a [`Prefs`] field.
#[component]
fn PrefSwitch(#[props(into)] label: String, pref: Pref) -> Element {
    let ctx = use_todo();
    let on = *pref.field(&mut ctx.prefs.read().clone());
    rsx! {
        div { class: "ax-todo-setting",
            Switch { checked: on, label, onchange: move |v| ctx.update_prefs(|p| *pref.field(p) = v) }
        }
    }
}

#[component]
fn SettingsDialog() -> Element {
    let mut ctx = use_todo();
    rsx! {
        Dialog { title: "To Do settings", width: 520, on_close: move |_| ctx.dialog.set(None),
            div { class: "ax-todo-settings",
                h3 { "General" }
                PrefSwitch { label: "Confirm before deleting", pref: Pref::ConfirmDelete }
                PrefSwitch { label: "Add new tasks on top", pref: Pref::AddOnTop }
                PrefSwitch { label: "Move starred tasks to top", pref: Pref::StarredOnTop }
                PrefSwitch { label: "Play completion sound", pref: Pref::Sound }
                PrefSwitch { label: "Turn on reminder notifications", pref: Pref::Reminders }
                h3 { "Smart lists" }
                PrefSwitch { label: "Important", pref: Pref::Important }
                PrefSwitch { label: "Planned", pref: Pref::Planned }
                PrefSwitch { label: "All", pref: Pref::All }
                PrefSwitch { label: "Completed", pref: Pref::Completed }
                PrefSwitch { label: "Assigned to me", pref: Pref::Assigned }
                PrefSwitch { label: "Auto-hide empty smart lists", pref: Pref::AutoHide }
                PrefSwitch { label: "Show tasks due today in My Day", pref: Pref::DueToday }
                h3 { "Connected apps" }
                PrefSwitch { label: "Flagged email", pref: Pref::Flagged }
                p { class: "ax-todo-settings-note",
                    "My Day, list groups and manual order are kept on this device and sync between Axentra installs, not to Outlook."
                }
            }
        }
    }
}

/// (action, keys) rows of the To Do shortcuts list (keyboard spec section 8).
const SHORTCUTS: [(&str, &str); 14] = [
    ("New task", "Ctrl+N"),
    ("New list", "Ctrl+L"),
    ("Add to My Day / remove from My Day", "Ctrl+T"),
    ("Mark complete / not complete", "Ctrl+D or Insert"),
    ("Search tasks", "Ctrl+E or Ctrl+F"),
    ("Sync now", "F9"),
    ("Add the task you typed", "Enter"),
    ("Open task details", "Enter"),
    ("Close details, cancel rename", "Esc"),
    ("Move between tasks", "Up / Down"),
    ("Move a task up or down", "Ctrl+Up / Ctrl+Down"),
    ("Delete task", "Delete"),
    ("Rename task or list", "F2"),
    ("Select several tasks", "Shift+click / Ctrl+click"),
];

#[component]
fn ShortcutsDialog() -> Element {
    let mut ctx = use_todo();
    rsx! {
        Dialog { title: "Keyboard shortcuts", width: 560, on_close: move |_| ctx.dialog.set(None),
            div { class: "ax-todo-shortcuts", role: "list",
                h3 { "To Do" }
                for (action, keys) in SHORTCUTS {
                    div { key: "{action}", class: "ax-todo-shortcut", role: "listitem", "aria-label": "{action}: {keys}",
                        span { class: "ax-grow", "{action}" }
                        span { class: "ax-todo-keys",
                            for (i, k) in keys.split(" or ").enumerate() {
                                if i > 0 { span { class: "ax-muted", " or " } }
                                kbd { class: "ax-todo-kbd", "{k}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
