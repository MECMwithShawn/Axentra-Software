//! "My Day" side pane (docs/spec/shell.md §3.4, people-todo.md §4): today's
//! agenda and today's tasks, docked at the right of the content.

mod agenda;
mod day_view;
mod mini_month;
mod todo;

use dioxus::prelude::*;

use crate::kit::{IconButton, Pivot, dom};
use crate::state::{store, use_app};

const TAB_KEY: &str = "shell.my_day_tab";

/// Shared container for the right-hand side panes (My Day, Notifications,
/// Help): title row with a close button, Escape closes, F6 stop.
#[component]
pub fn SidePaneFrame(
    #[props(into)] title: String,
    #[props(into)] close_label: String,
    on_close: EventHandler<()>,
    #[props(into, default)] class: String,
    /// Extra controls in the title row, before the close button.
    tools: Option<Element>,
    children: Element,
) -> Element {
    rsx! {
        aside {
            class: "ax-sidepanel {class}",
            role: "complementary",
            "aria-label": "{title}",
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape {
                    e.stop_propagation();
                    on_close.call(());
                }
            },
            div { class: "ax-sidepanel-head",
                h2 { class: "ax-sidepanel-title", "{title}" }
                if let Some(t) = tools { {t} }
                IconButton { icon: "dismiss", title: "{close_label}", onclick: move |_| on_close.call(()) }
            }
            {children}
        }
    }
}

#[component]
pub fn MyDayPane() -> Element {
    let mut app = use_app();
    let mut tab = use_signal(|| store().kv_get(TAB_KEY).ok().flatten().unwrap_or_else(|| "calendar".into()));
    let mut mail_hover = use_signal(|| false);
    let mut last_over = use_signal(std::time::Instant::now);
    let close = move |_| {
        app.my_day_open.set(false);
        dom::focus(".ax-header-end [aria-label='My Day']");
    };
    rsx! {
        SidePaneFrame { title: "My Day", close_label: "Close My Day", class: "ax-my-day", on_close: close,
            Pivot {
                class: "ax-sidepanel-tabs",
                label: "My Day",
                items: vec![("calendar".into(), "Calendar".into()), ("todo".into(), "To Do".into())],
                selected: tab(),
                onselect: move |k: String| {
                    let _ = store().kv_set(TAB_KEY, &k);
                    tab.set(k);
                },
            }
            if tab() == "todo" {
                todo::TodoTab {}
            } else {
                // Mail dragged over the Calendar tab switches to To Do after
                // 600 ms, so it can be dropped there (people-todo.md 4.3).
                div {
                    class: "ax-myday-calendar-drop",
                    ondragover: move |e: DragEvent| {
                        if todo::dragged_messages(&e).is_none() {
                            return;
                        }
                        last_over.set(std::time::Instant::now());
                        if !*mail_hover.peek() {
                            mail_hover.set(true);
                            spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                                mail_hover.set(false);
                                // Still hovering (dragover repeats while the pointer is here).
                                if last_over.peek().elapsed() < std::time::Duration::from_millis(300) {
                                    tab.set("todo".into());
                                }
                            });
                        }
                    },
                    agenda::CalendarTab {}
                }
            }
        }
    }
}
