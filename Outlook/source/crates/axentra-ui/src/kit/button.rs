//! Icon, Button and IconButton (Fluent v9 button appearances).

use dioxus::prelude::*;

use crate::icons::icon_svg;

const FALLBACK_SVG: &str = r#"<svg viewBox="0 0 20 20" xmlns="http://www.w3.org/2000/svg"><rect x="4" y="4" width="12" height="12" rx="2" fill="none" stroke="currentColor"/></svg>"#;

/// The `_filled` variant of an icon when one exists (Fluent uses the filled
/// style for selected / checked states), else the icon itself.
pub fn filled_icon(name: &str) -> String {
    if name.ends_with("_filled") {
        return name.to_string();
    }
    let filled = format!("{name}_filled");
    if icon_svg(&filled).is_some() { filled } else { name.to_string() }
}

/// A Fluent System Icon by file name (see `crates/axentra-ui/icons/`).
#[component]
pub fn Icon(#[props(into)] name: String, #[props(default = 20)] size: u32, #[props(into, default)] class: String) -> Element {
    let svg = icon_svg(&name).unwrap_or(FALLBACK_SVG);
    rsx! {
        span {
            class: "ax-icon {class}",
            style: "width:{size}px;height:{size}px",
            "aria-hidden": "true",
            dangerous_inner_html: svg,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Variant {
    #[default]
    Secondary,
    Primary,
    Subtle,
    Transparent,
    Danger,
}

impl Variant {
    pub(crate) fn class(self) -> &'static str {
        match self {
            Variant::Secondary => "",
            Variant::Primary => "ax-primary",
            Variant::Subtle => "ax-subtle",
            Variant::Transparent => "ax-transparent",
            Variant::Danger => "ax-danger",
        }
    }
}

/// Text button with an optional leading icon. Disabled buttons stay
/// hoverable so their tooltip (`title`) can explain why.
#[component]
pub fn Button(
    #[props(into)] label: String,
    #[props(into)] icon: Option<String>,
    #[props(default)] variant: Variant,
    #[props(default)] disabled: bool,
    #[props(default)] small: bool,
    #[props(into)] title: Option<String>,
    #[props(into, default)] class: String,
    onclick: Option<EventHandler<MouseEvent>>,
    /// Toggle button state (`aria-pressed`); `None` for plain buttons.
    pressed: Option<bool>,
    /// Receive focus when the surrounding dialog opens.
    #[props(default)]
    autofocus: bool,
    /// `aria-keyshortcuts` value, e.g. "Control+N" (also shown in the tooltip).
    #[props(into)]
    keyshortcuts: Option<String>,
    #[props(into)] id: Option<String>,
    /// The surrounding dialog's default button: Enter presses it while focus
    /// is in a text field or elsewhere outside buttons, menus and lists.
    #[props(default)]
    default_action: bool,
) -> Element {
    let v = variant.class();
    let s = if small { "ax-small" } else { "" };
    let p = if pressed == Some(true) { "ax-toggled" } else { "" };
    rsx! {
        button {
            r#type: "button",
            id,
            class: "ax-btn {v} {s} {p} {class}",
            "aria-disabled": if disabled { "true" },
            "aria-pressed": pressed.map(|p| if p { "true" } else { "false" }),
            "aria-keyshortcuts": keyshortcuts,
            "data-ax-autofocus": if autofocus { "true" },
            "data-ax-default": if default_action { "true" },
            title: title.clone().unwrap_or_default(),
            onclick: move |e| {
                if disabled {
                    return;
                }
                if let Some(h) = &onclick { h.call(e) }
            },
            if let Some(i) = &icon { Icon { name: i.clone() } }
            if !label.is_empty() { span { class: "ax-btn-label", "{label}" } }
        }
    }
}

/// Icon-only button. `title` is both the tooltip and the accessible name.
/// A toggled button shows the filled icon variant.
#[component]
pub fn IconButton(
    #[props(into)] icon: String,
    #[props(into)] title: String,
    #[props(default)] disabled: bool,
    #[props(default)] toggled: bool,
    #[props(default)] small: bool,
    #[props(into, default)] class: String,
    onclick: Option<EventHandler<MouseEvent>>,
    /// Announce as a toggle button (`aria-pressed`) even while not toggled.
    #[props(default)]
    toggle: bool,
    #[props(into)] keyshortcuts: Option<String>,
    /// Tooltip placement: "below" (default) or "right".
    #[props(into)]
    tip_pos: Option<String>,
    #[props(into)] id: Option<String>,
    #[props(default)] autofocus: bool,
) -> Element {
    let t = if toggled { "ax-toggled" } else { "" };
    let s = if small { "ax-small" } else { "" };
    let shown = if toggled { filled_icon(&icon) } else { icon.clone() };
    rsx! {
        button {
            r#type: "button",
            id,
            class: "ax-icon-btn {t} {s} {class}",
            title: "{title}",
            "aria-label": "{title}",
            "aria-pressed": if toggle || toggled { if toggled { "true" } else { "false" } },
            "aria-disabled": if disabled { "true" },
            "aria-keyshortcuts": keyshortcuts,
            "data-ax-tip-pos": tip_pos,
            "data-ax-autofocus": if autofocus { "true" },
            onclick: move |e| {
                e.stop_propagation();
                if disabled {
                    return;
                }
                if let Some(h) = &onclick { h.call(e) }
            },
            Icon { name: shown }
        }
    }
}
