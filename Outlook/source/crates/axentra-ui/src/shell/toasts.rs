//! Toasts and the undo bar (docs/spec/shell.md §8.1, §8.3): bottom-left of
//! the content area, newest at the bottom, at most three. Timers pause
//! while the pointer or focus is on a toast; errors stay until dismissed.
//! A countdown toast (Undo send, mail spec 6.9) shows a spinner and a
//! draining bar and closes on time, hovered or not.

use dioxus::prelude::*;

use super::use_shell;
use crate::kit::{Icon, IconButton, Intent};
use crate::state::{Toast, use_app};

/// How long a toast stays, in ms (`None` = until dismissed).
pub fn lifetime(t: &Toast) -> Option<u64> {
    if let Some(c) = t.countdown {
        return Some(c.total_ms);
    }
    match intent(t) {
        Intent::Error => None,
        Intent::Success => Some(6_000),
        Intent::Warning => Some(8_000),
        Intent::Info => Some(6_000),
    }
}

pub fn intent(t: &Toast) -> Intent {
    if t.error {
        Intent::Error
    } else if t.undo.is_some() {
        Intent::Success
    } else {
        Intent::Info
    }
}

#[component]
pub fn Toasts() -> Element {
    let app = use_app();
    let toasts = app.toasts.read().clone();
    rsx! {
        div { class: "ax-toasts", role: "region", "aria-label": "Notifications and undo",
            div { role: "status", "aria-live": "polite", class: "ax-toast-stack",
                for t in toasts.iter().filter(|t| !t.error).cloned() {
                    ToastItem { key: "{t.id}", toast: t }
                }
            }
            div { role: "alert", class: "ax-toast-stack",
                for t in toasts.iter().filter(|t| t.error).cloned() {
                    ToastItem { key: "{t.id}", toast: t }
                }
            }
        }
    }
}

#[component]
fn ToastItem(toast: Toast) -> Element {
    let app = use_app();
    let mut shell = use_shell();
    let id = toast.id;
    let mut hovered = use_signal(|| false);
    let mut focused = use_signal(|| false);
    let life = lifetime(&toast);
    let countdown = toast.countdown;
    use_future(move || async move {
        if let Some(c) = countdown {
            // Ends on time: the held message goes out at `until` regardless.
            let left = (c.until - chrono::Utc::now()).num_milliseconds().max(0) as u64;
            tokio::time::sleep(std::time::Duration::from_millis(left)).await;
            app.dismiss_toast(id);
            return;
        }
        let Some(total) = life else { return };
        let mut left = total as i64;
        while left > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if !hovered() && !focused() {
                left -= 100;
            }
        }
        app.dismiss_toast(id);
    });
    let kind = intent(&toast);
    // The bar starts at the share of the delay still left (a toast can be
    // shown after part of it ran) and drains to zero in the remaining time.
    let bar = countdown.map(|c| {
        let left = (c.until - chrono::Utc::now()).num_milliseconds().max(0) as u64;
        let start = if c.total_ms == 0 { 0.0 } else { left as f64 / c.total_ms as f64 };
        format!("--ax-countdown-start:{start:.3};animation-duration:{left}ms")
    });
    rsx! {
        div {
            class: "ax-toast {kind.class()}",
            class: if toast.undo.is_some() { "ax-undo-bar" },
            class: if countdown.is_some() { "ax-toast-countdown" },
            onmouseenter: move |_| hovered.set(true),
            onmouseleave: move |_| hovered.set(false),
            onfocusin: move |_| focused.set(true),
            onfocusout: move |_| focused.set(false),
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape {
                    e.stop_propagation();
                    app.dismiss_toast(id);
                }
            },
            if countdown.is_some() {
                // The toast text says what's happening; the spinner is decoration.
                span { class: "ax-spinner ax-toast-spinner", "aria-hidden": "true", style: "width:16px;height:16px;border-width:2px" }
            } else {
                Icon { name: kind.icon(), size: 16, class: "ax-toast-icon" }
            }
            span { class: "ax-toast-text", "{toast.text}" }
            if let Some((label, action)) = toast.action.clone() {
                button {
                    r#type: "button",
                    class: "ax-toast-action",
                    onclick: move |_| {
                        app.run_toast_action(&action);
                        app.dismiss_toast(id);
                    },
                    "{label}"
                }
            }
            if let Some(undo) = toast.undo.clone() {
                button {
                    r#type: "button",
                    class: "ax-toast-action",
                    "aria-keyshortcuts": "Control+Z",
                    onclick: move |_| {
                        app.run_undo(&undo);
                        shell.undo.write().retain(|(t, _)| *t != id);
                        app.dismiss_toast(id);
                        crate::kit::dom::announce("Undone", false);
                    },
                    "Undo"
                }
            }
            IconButton { icon: "dismiss", title: "Dismiss", small: true, onclick: move |_| app.dismiss_toast(id) }
            if let Some(style) = bar {
                span { class: "ax-toast-progress", "aria-hidden": "true",
                    span { class: "ax-toast-progress-fill", style: "{style}" }
                }
            }
        }
    }
}
