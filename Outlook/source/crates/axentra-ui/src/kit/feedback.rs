//! Feedback and status: EmptyState, Spinner, MessageBar, CounterBadge,
//! ProgressBar and Tooltip.

use dioxus::prelude::*;

use super::button::{Icon, IconButton};

/// Status intent of a message bar or toast (Fluent v9 mapping).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Intent {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

impl Intent {
    pub fn icon(self) -> &'static str {
        match self {
            Intent::Info => "info",
            Intent::Success => "checkmark_circle",
            Intent::Warning => "warning",
            Intent::Error => "error_circle",
        }
    }

    pub fn class(self) -> &'static str {
        match self {
            Intent::Info => "ax-info",
            Intent::Success => "ax-success",
            Intent::Warning => "ax-warning",
            Intent::Error => "ax-error",
        }
    }
}

/// Empty / zero-data state: 24px icon, Subtitle 2 title, Body 1 text.
#[component]
pub fn EmptyState(
    #[props(into)] icon: String,
    #[props(into)] title: String,
    #[props(into, default)] subtitle: String,
    children: Option<Element>,
    #[props(default = 24)] icon_size: u32,
) -> Element {
    rsx! {
        div { class: "ax-empty",
            Icon { name: icon.clone(), size: icon_size }
            div { class: "ax-empty-title", "{title}" }
            if !subtitle.is_empty() { div { class: "ax-empty-text", "{subtitle}" } }
            if let Some(c) = children { {c} }
        }
    }
}

/// Fluent Spinner. `size` is 16 (extra tiny) to 44 (huge); 32 in panes,
/// 20 inline.
#[component]
pub fn Spinner(#[props(into, default)] label: String, #[props(default = 20)] size: u32) -> Element {
    let stroke = match size {
        0..=28 => 2,
        29..=40 => 3,
        _ => 4,
    };
    rsx! {
        div { class: "ax-row ax-spinner-wrap", role: "status",
            div { class: "ax-spinner", style: "width:{size}px;height:{size}px;border-width:{stroke}px" }
            if !label.is_empty() { span { "{label}" } }
            if label.is_empty() { span { class: "ax-visually-hidden", "Loading" } }
        }
    }
}

/// Inline, persistent notice (Fluent MessageBar).
#[component]
pub fn MessageBar(
    #[props(default)] intent: Intent,
    #[props(into, default)] title: String,
    #[props(into, default)] text: String,
    /// Action buttons on the right.
    actions: Option<Element>,
    on_dismiss: Option<EventHandler<()>>,
    #[props(into, default)] class: String,
) -> Element {
    let role = if intent == Intent::Error { "alert" } else { "note" };
    rsx! {
        div { class: "ax-message-bar {intent.class()} {class}", role,
            Icon { name: intent.icon(), size: 16, class: "ax-message-bar-icon" }
            div { class: "ax-message-bar-text",
                if !title.is_empty() { span { class: "ax-semibold", "{title} " } }
                "{text}"
            }
            if let Some(a) = actions { div { class: "ax-message-bar-actions", {a} } }
            if let Some(h) = on_dismiss {
                IconButton { icon: "dismiss", title: "Dismiss", small: true, onclick: move |_| h.call(()) }
            }
        }
    }
}

/// Count badge (notifications, unread). Hidden at 0, "99+" above 99.
#[component]
pub fn CounterBadge(count: usize, #[props(into, default)] class: String, #[props(into, default)] label: String) -> Element {
    if count == 0 {
        return rsx! {};
    }
    let text = if count > 99 { "99+".to_string() } else { count.to_string() };
    let aria = if label.is_empty() { format!("{count}") } else { label.replace("{n}", &count.to_string()) };
    rsx! {
        span { class: "ax-counter-badge {class}", "aria-label": "{aria}", role: "status", "{text}" }
    }
}

/// Thin progress bar; `value` in 0..=1, `None` = indeterminate.
#[component]
pub fn ProgressBar(value: Option<f64>, #[props(into, default)] label: String) -> Element {
    rsx! {
        div {
            class: "ax-progress",
            class: if value.is_none() { "ax-indeterminate" },
            role: "progressbar",
            "aria-label": "{label}",
            "aria-valuemin": "0",
            "aria-valuemax": "100",
            "aria-valuenow": value.map(|v| format!("{:.0}", v.clamp(0.0, 1.0) * 100.0)),
            div { class: "ax-progress-bar", style: value.map(|v| format!("width:{:.1}%", v.clamp(0.0, 1.0) * 100.0)) }
        }
    }
}

/// Styled tooltip for any content (shown after 500 ms of hover, and on
/// keyboard focus of icon-only children). Plain `title` attributes get the
/// same tooltip automatically; use this for non-button content.
#[component]
pub fn Tooltip(#[props(into)] content: String, children: Element, #[props(default)] right: bool) -> Element {
    rsx! {
        span {
            class: "ax-tooltip-host",
            "data-ax-tip": "{content}",
            "data-ax-tip-pos": if right { "right" },
            {children}
        }
    }
}
