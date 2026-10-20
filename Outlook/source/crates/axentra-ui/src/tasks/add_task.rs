//! "Add a task" input (3.5) with its quick due date / reminder / repeat
//! toolbar.

use axentra_core::*;
use dioxus::prelude::*;

use super::actions;
use super::ctx::{Draft, Target, View, use_todo};
use super::dates;
use super::menus::{self, MenuAnchor};
use super::task_row::CHECK_SVG;
use crate::kit::{Button, Dialog, Icon, Variant, keys};
use crate::state::use_app;

/// Title length limit; the counter shows from [`COUNTER_FROM`].
const MAX_TITLE: usize = 255;
const COUNTER_FROM: usize = 230;

/// Element id of the input, focused by Ctrl+N and the ribbon.
pub const ADD_INPUT_ID: &str = "ax-todo-add-input";

/// Focus "Add a task" (after the next render, in case the view changes).
pub fn focus_add() {
    actions::focus_later(&format!("#{ADD_INPUT_ID}"), 40);
}

/// Watches pastes into the input: a paste of several non-empty lines is
/// held back and sent to Rust, which asks "Add {N} tasks?" (spec 3.5).
/// Single-line pastes go into the input as usual.
const PASTE_JS: &str = r#"
const attach = () => {
  const el = document.getElementById("ax-todo-add-input");
  if (!el) { setTimeout(attach, 50); return; }
  el.__axPasteSend = (t) => dioxus.send(t);
  if (el.__axPaste) return;
  el.__axPaste = true;
  el.addEventListener("paste", (e) => {
    const text = (e.clipboardData && e.clipboardData.getData("text")) || "";
    if (text.split(/\r?\n/).filter((l) => l.trim()).length > 1) {
      e.preventDefault();
      el.__axPasteSend(text);
    }
  });
};
attach();
"#;

/// Non-empty, trimmed lines of a paste, each cut to the title limit.
pub fn paste_lines(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| l.chars().take(MAX_TITLE).collect()).collect()
}

#[component]
pub fn AddTaskBar(view: View, list_id: Option<TaskListId>, #[props(into)] list_name: String) -> Element {
    let app = use_app();
    let mut ctx = use_todo();
    let mut text = use_signal(String::new);
    let mut focused = use_signal(|| false);
    let mut focus_token = use_signal(|| 0u64);
    let mut menu_open = use_signal(|| false);
    // Lines of a multi-line paste waiting for "Add {N} tasks?".
    let mut pasted = use_signal(Vec::<String>::new);
    use_future(move || async move {
        let mut watch = document::eval(PASTE_JS);
        while let Ok(text) = watch.recv::<String>().await {
            let lines = paste_lines(&text);
            if lines.len() > 1 {
                pasted.set(lines);
            }
        }
    });
    let draft = ctx.draft.read().clone();
    let today = (ctx.today)();
    let h24 = app.settings.read().use_24h_time;
    let len = text().chars().count();
    let active = focused() || !text().is_empty() || draft != Draft::default() || menu_open();
    let empty = text().trim().is_empty();
    let mut commit = move || {
        let title = text().trim().to_string();
        if !title.is_empty() && actions::add_task(app, ctx, &title, list_id, view).is_some() {
            text.set(String::new());
        }
    };
    let run = move |id: String| actions::run(app, ctx, Target::NewTask, &id);
    rsx! {
        div {
            class: "ax-todo-add",
            class: if active { "ax-active" },
            onfocusin: move |_| {
                focus_token += 1;
                focused.set(true);
            },
            onfocusout: move |_| {
                // Focus may be moving to a toolbar button; decide a moment later.
                let token = *focus_token.peek();
                ctx.spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    if *focus_token.peek() == token {
                        focused.set(false);
                    }
                });
            },
            label { class: "ax-todo-add-row",
                span { class: "ax-todo-add-icon", "aria-hidden": "true",
                    if active {
                        span { class: "ax-todo-check-svg", dangerous_inner_html: CHECK_SVG }
                    } else {
                        Icon { name: "add" }
                    }
                }
                input {
                    id: ADD_INPUT_ID,
                    class: "ax-todo-add-input",
                    r#type: "text",
                    placeholder: "Add a task",
                    "aria-label": "Add a task in {list_name}",
                    "aria-keyshortcuts": "Control+N",
                    maxlength: "{MAX_TITLE}",
                    autocomplete: "off",
                    value: "{text}",
                    oninput: move |e| text.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        keys::stop_typing_keys(&e);
                        match e.key() {
                            Key::Enter => {
                                e.prevent_default();
                                commit();
                            }
                            Key::Escape => {
                                e.stop_propagation();
                                if text().is_empty() && *ctx.draft.peek() == Draft::default() {
                                    // Second Esc leaves the input for the list (its roving tab stop).
                                    document::eval(
                                        "const r = document.querySelector('.ax-todo-grid [data-task-row][tabindex=\"0\"]'); if (r) r.focus(); else document.activeElement?.blur();",
                                    );
                                } else {
                                    text.set(String::new());
                                    ctx.draft.set(Draft::default());
                                }
                            }
                            _ => {}
                        }
                    },
                }
                if len >= COUNTER_FROM {
                    span { class: "ax-todo-add-count", "aria-live": "polite", "{len}/{MAX_TITLE}" }
                }
            }
            if active {
                div { class: "ax-todo-add-tools",
                    if let Some(due) = draft.due {
                        QuickChip {
                            icon: "calendar_ltr",
                            label: dates::due_row_label(due, today),
                            items: menus::due_menu(today, true),
                            remove_label: "Remove due date",
                            on_select: run,
                            on_remove: move |_| ctx.draft.write().due = None,
                            on_open: move |o| menu_open.set(o),
                        }
                    } else {
                        MenuAnchor {
                            class: "ax-icon-btn",
                            title: "Add due date",
                            items: menus::due_menu(today, false),
                            on_select: run,
                            on_open: move |o| menu_open.set(o),
                            Icon { name: "calendar_ltr" }
                        }
                    }
                    if let Some(r) = draft.reminder {
                        QuickChip {
                            icon: "alert",
                            label: {
                                let (_, day) = dates::reminder_lines(r, today, h24);
                                format!("{day} {}", crate::fmt::time(r, h24))
                            },
                            items: menus::reminder_menu(today, true, h24),
                            remove_label: "Remove reminder",
                            on_select: run,
                            on_remove: move |_| ctx.draft.write().reminder = None,
                            on_open: move |o| menu_open.set(o),
                        }
                    } else {
                        MenuAnchor {
                            class: "ax-icon-btn",
                            title: "Remind me",
                            items: menus::reminder_menu(today, false, h24),
                            on_select: run,
                            on_open: move |o| menu_open.set(o),
                            Icon { name: "alert" }
                        }
                    }
                    if let Some(rule) = draft.repeat.clone() {
                        QuickChip {
                            icon: "arrow_repeat_all",
                            label: dates::repeat_label(&rule),
                            items: menus::repeat_menu(true),
                            remove_label: "Remove recurrence",
                            on_select: run,
                            on_remove: move |_| ctx.draft.write().repeat = None,
                            on_open: move |o| menu_open.set(o),
                        }
                    } else {
                        MenuAnchor {
                            class: "ax-icon-btn",
                            title: "Repeat",
                            items: menus::repeat_menu(false),
                            on_select: run,
                            on_open: move |o| menu_open.set(o),
                            Icon { name: "arrow_repeat_all" }
                        }
                    }
                    span { class: "ax-grow" }
                    Button {
                        label: "Add",
                        small: true,
                        variant: if empty { Variant::Secondary } else { Variant::Primary },
                        disabled: empty,
                        title: if empty { "Type a task name first" } else { "Add task (Enter)" },
                        onclick: move |_| commit(),
                    }
                }
            }
            if !pasted().is_empty() {
                PasteDialog {
                    lines: pasted(),
                    on_many: move |_| {
                        let lines = pasted();
                        pasted.set(Vec::new());
                        actions::add_tasks(app, ctx, &lines, list_id, view);
                        focus_add();
                    },
                    on_one: move |_| {
                        let one: String = pasted().join(" ").chars().take(MAX_TITLE).collect();
                        pasted.set(Vec::new());
                        actions::add_task(app, ctx, &one, list_id, view);
                        focus_add();
                    },
                    on_cancel: move |_| {
                        pasted.set(Vec::new());
                        focus_add();
                    },
                }
            }
        }
    }
}

/// "Add {N} tasks?" after pasting several lines.
#[component]
fn PasteDialog(lines: Vec<String>, on_many: EventHandler<()>, on_one: EventHandler<()>, on_cancel: EventHandler<()>) -> Element {
    let n = lines.len();
    let more = n.saturating_sub(5);
    rsx! {
        Dialog {
            title: "Add {n} tasks?",
            width: 400,
            on_close: move |_| on_cancel.call(()),
            actions: rsx! {
                Button { label: "Add {n} tasks", variant: Variant::Primary, default_action: true, onclick: move |_| on_many.call(()) }
                Button { label: "Add as one task", onclick: move |_| on_one.call(()) }
            },
            p { class: "ax-todo-dialog-text", "Each line you pasted can become its own task:" }
            ul { class: "ax-todo-paste-list",
                for (i, l) in lines.iter().take(5).enumerate() {
                    li { key: "{i}", class: "ax-ellipsis", "{l}" }
                }
                if more > 0 {
                    li { class: "ax-todo-paste-more", "and {more} more" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::paste_lines;

    #[test]
    fn pasted_lines_are_trimmed_and_capped() {
        assert_eq!(paste_lines("Milk\r\n\n  Eggs  \nBread\n"), ["Milk", "Eggs", "Bread"]);
        assert_eq!(paste_lines(&"x".repeat(300))[0].len(), 255);
        assert!(paste_lines(" \n \n").is_empty());
    }
}

/// A set quick value ("Due Tomorrow") that reopens its menu, with an x.
#[component]
fn QuickChip(
    icon: &'static str,
    #[props(into)] label: String,
    items: Vec<crate::kit::MenuItem>,
    remove_label: &'static str,
    on_select: EventHandler<String>,
    on_remove: EventHandler<()>,
    on_open: EventHandler<bool>,
) -> Element {
    rsx! {
        span { class: "ax-todo-quick-chip",
            MenuAnchor { class: "ax-todo-quick-chip-btn", title: "{label}", items, on_select, on_open,
                Icon { name: icon, size: 16 }
                span { "{label}" }
            }
            button {
                r#type: "button",
                class: "ax-todo-quick-chip-x",
                title: "{remove_label}",
                "aria-label": "{remove_label}",
                onclick: move |_| on_remove.call(()),
                Icon { name: "dismiss", size: 12 }
            }
        }
    }
}
