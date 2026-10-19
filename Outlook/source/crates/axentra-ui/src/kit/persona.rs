//! People and categories: Avatar (with optional presence), CategoryPill.

use dioxus::prelude::*;

use super::button::Icon;

/// Presence shown as a badge on an [`Avatar`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Available,
    Busy,
    Away,
    DoNotDisturb,
    Offline,
}

impl Presence {
    fn class(self) -> &'static str {
        match self {
            Presence::Available => "ax-available",
            Presence::Busy => "ax-busy",
            Presence::Away => "ax-away",
            Presence::DoNotDisturb => "ax-dnd",
            Presence::Offline => "ax-offline",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Presence::Available => "Available",
            Presence::Busy => "Busy",
            Presence::Away => "Away",
            Presence::DoNotDisturb => "Do not disturb",
            Presence::Offline => "Offline",
        }
    }
}

/// Initials avatar in the persona palette, coloured deterministically from
/// the email (or name).
#[component]
pub fn Avatar(
    #[props(into)] name: String,
    #[props(into, default)] email: String,
    #[props(default = 32)] size: u32,
    #[props(into, default)] class: String,
    presence: Option<Presence>,
) -> Element {
    let key = if email.is_empty() { &name } else { &email };
    let idx = axentra_core::text::persona_color_index(key);
    let initials = axentra_core::text::initials(if name.trim().is_empty() { &email } else { &name });
    let font = (size as f64 * 0.4).round().max(9.0);
    let badge = (size as f64 * 0.3).round().clamp(6.0, 16.0);
    rsx! {
        span {
            class: "ax-avatar ax-persona-{idx} {class}",
            style: "width:{size}px;height:{size}px;font-size:{font}px",
            title: if email.is_empty() { name.clone() } else { format!("{name} <{email}>") },
            "aria-hidden": "true",
            if initials.is_empty() { Icon { name: "person", size: (size as f64 * 0.6) as u32 } } else { "{initials}" }
            if let Some(p) = presence {
                span { class: "ax-presence {p.class()}", style: "width:{badge}px;height:{badge}px", title: p.label() }
            }
        }
    }
}

/// Category colour pill (as shown on messages and events).
#[component]
pub fn CategoryPill(#[props(into)] name: String, #[props(into)] color: String) -> Element {
    let (light, _dark) = category_colors(&color);
    rsx! {
        span { class: "ax-category-pill", style: "background:color-mix(in srgb, {light} 30%, transparent);border-left:3px solid {light}", "{name}" }
    }
}

/// (light, dark) hex for a preset category colour name; falls back to gray.
pub fn category_colors(name: &str) -> (&'static str, &'static str) {
    axentra_core::CATEGORY_COLORS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, l, d)| (*l, *d))
        .unwrap_or(("#7a7574", "#a19f9d"))
}

/// CSS for the persona colour classes `.ax-persona-0..N`, both themes.
pub fn persona_css() -> String {
    let mut css = String::new();
    for (i, (light, dark)) in axentra_core::text::PERSONA_COLORS.iter().enumerate() {
        css.push_str(&format!(
            ".ax-app .ax-persona-{i}{{background:{light}}}.ax-app.ax-theme-dark .ax-persona-{i}{{background:{dark}}}\
             @media (prefers-color-scheme: dark){{.ax-app.ax-theme-system .ax-persona-{i}{{background:{dark}}}}}"
        ));
    }
    css
}
