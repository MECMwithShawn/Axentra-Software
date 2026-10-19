//! The action bar (`[Send ▾] [Discard] … "Draft saved at 3:42 PM"`) and the
//! banners above the body.

use std::rc::Rc;

use dioxus::prelude::*;

use super::actions::{self, SIZE_LIMIT};
use super::commands;
use super::state::{Form, SaveStatus};
use crate::fmt;
use crate::kit::{Button, Icon, Menu, MenuItem, Variant, keys};
use crate::state::{store, use_app};

/// Inline compose shows it at the bottom; the pop-out window at the top
/// (mail spec 6.2).
#[component]
pub fn ActionBar(#[props(default)] top: bool) -> Element {
    let app = use_app();
    let form = use_context::<Form>();
    let status = (form.status)();
    let h24 = app.settings.read().use_24h_time;
    rsx! {
        div { class: "ax-compose-actions", class: if top { "ax-top" },
            SendSplit { above: !top }
            Button {
                label: "Discard",
                icon: "delete",
                variant: Variant::Subtle,
                title: "Discard",
                onclick: move |_| { form.spawn(actions::discard(app, form)); },
            }
            div { class: "ax-grow" }
            div { class: "ax-compose-status", "aria-live": "off",
                match status {
                    SaveStatus::Idle => rsx! {},
                    SaveStatus::Saved(t) => rsx! { "Draft saved at {fmt::time(t.with_timezone(&chrono::Utc), h24)}" },
                    SaveStatus::Failed(reason) => rsx! {
                        span { class: "ax-compose-status-error", title: "{reason}", "Couldn't save draft" }
                        button {
                            r#type: "button",
                            class: "ax-compose-link-btn",
                            onclick: move |_| { form.spawn(commands::save(app, form)); },
                            "Retry"
                        }
                    },
                }
            }
        }
    }
}

/// Height of the Send ▾ menu: two 32px items, 4px padding, 1px border.
const SEND_MENU_HEIGHT: f64 = 2.0 * 32.0 + 8.0 + 2.0;

#[component]
fn SendSplit(above: bool) -> Element {
    let app = use_app();
    let form = use_context::<Form>();
    let busy = (form.busy)();
    let mut menu = use_signal(|| None::<(f64, f64)>);
    let mut anchor = use_signal(|| None::<Rc<MountedData>>);
    let open_menu = move || {
        spawn(async move {
            let rect = match anchor() {
                Some(m) => m.get_client_rect().await.ok(),
                None => None,
            };
            // At the bottom of the form the menu opens upwards (two 32px items).
            let y = |r: &dioxus::html::geometry::PixelsRect| if above { r.min_y() - 4.0 - SEND_MENU_HEIGHT } else { r.max_y() + 4.0 };
            menu.set(Some(rect.map(|r| (r.min_x(), y(&r))).unwrap_or((0.0, 0.0))));
        });
    };
    rsx! {
        div {
            class: "ax-split ax-primary ax-compose-send",
            role: "group",
            "aria-label": "Send",
            onmounted: move |e| anchor.set(Some(e.data())),
            button {
                r#type: "button",
                class: "ax-btn ax-primary",
                title: "Send (Ctrl+Enter)",
                "aria-keyshortcuts": "Control+Enter",
                disabled: busy,
                onclick: move |_| { form.spawn(actions::send(app, form)); },
                onkeydown: move |e: KeyboardEvent| {
                    if keys::chord(&e) == "Alt+ArrowDown" {
                        e.prevent_default();
                        e.stop_propagation();
                        open_menu();
                    }
                },
                span { "Send" }
            }
            button {
                r#type: "button",
                class: "ax-btn ax-primary ax-split-chevron",
                title: "More send options",
                "aria-label": "More send options",
                "aria-haspopup": "menu",
                "aria-expanded": if menu().is_some() { "true" } else { "false" },
                disabled: busy,
                onclick: move |_| open_menu(),
                Icon { name: "chevron_down", size: 12 }
            }
        }
        if let Some((x, y)) = menu() {
            Menu {
                items: vec![
                    MenuItem::new("compose.send", "Send").icon("send").shortcut("Ctrl+Enter"),
                    MenuItem::new("compose.schedule", "Schedule send").icon("send_clock"),
                ],
                x,
                y,
                on_close: move |_| menu.set(None),
                on_select: move |id: String| commands::run(app, form, &id),
            }
        }
    }
}

/// "Scheduled" and "too large" banners.
#[component]
pub fn Banners() -> Element {
    let app = use_app();
    let mut form = use_context::<Form>();
    let d = form.draft.read();
    let scheduled = d.scheduled_send_at;
    let size = actions::attachments_size(&d);
    drop(d);
    let h24 = app.settings.read().use_24h_time;
    rsx! {
        if let Some(at) = scheduled {
            div { class: "ax-message-bar ax-compose-banner", role: "note",
                Icon { name: "send_clock", size: 16 }
                span { class: "ax-grow", "This message will be sent {fmt::full_time(at, h24)}." }
                button {
                    r#type: "button",
                    class: "ax-compose-link-btn",
                    onclick: move |_| {
                        let mut d = actions::current(form);
                        match store().cancel_scheduled_send(&mut d) {
                            Ok(()) => {
                                form.draft.set(d);
                                form.saved.set(*form.dirty.peek());
                                app.bump_mail();
                                app.toast("Scheduled send canceled");
                            }
                            Err(e) => { app.error(format!("Couldn't cancel the scheduled send: {e}")); }
                        }
                    },
                    "Cancel send"
                }
            }
        }
        if size > SIZE_LIMIT {
            div { class: "ax-message-bar ax-error ax-compose-banner", role: "alert",
                Icon { name: "error_circle", size: 16 }
                span { "This message is too large to send ({fmt::file_size(size)}). The limit for this account is {fmt::file_size(SIZE_LIMIT)}." }
            }
        }
    }
}
