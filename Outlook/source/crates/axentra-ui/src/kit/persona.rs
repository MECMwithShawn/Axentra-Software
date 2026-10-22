//! People and categories: Avatar (with optional presence and photo), CategoryPill.

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

/// The photo an avatar may show: only `data:image/…` URLs (the same rule as
/// the People avatars), so nothing is ever loaded from the network.
pub fn photo_src(photo: Option<&str>) -> Option<&str> {
    photo.filter(|p| p.starts_with("data:image/"))
}

/// The photo an avatar shows: [`photo_src`], unless that very photo failed
/// to load (`broken`), when the initials come back until the photo changes.
fn shown_photo<'a>(photo: Option<&'a str>, broken: Option<&str>) -> Option<&'a str> {
    photo_src(photo).filter(|p| broken != Some(*p))
}

/// Initials avatar in the persona palette, coloured deterministically from
/// the email (or name). With a `photo` (the user's own account photo) the
/// picture fills the same circle; initials are the fallback, also when the
/// picture can't be shown (bytes that look like an image but don't decode).
#[component]
pub fn Avatar(
    #[props(into)] name: String,
    #[props(into, default)] email: String,
    #[props(default = 32)] size: u32,
    #[props(into, default)] class: String,
    presence: Option<Presence>,
    /// A `data:image/…` URL; anything else is ignored.
    #[props(default)]
    photo: Option<String>,
) -> Element {
    let key = if email.is_empty() { &name } else { &email };
    let idx = axentra_core::text::persona_color_index(key);
    let initials = axentra_core::text::initials(if name.trim().is_empty() { &email } else { &name });
    let font = (size as f64 * 0.4).round().max(9.0);
    let badge = (size as f64 * 0.3).round().clamp(6.0, 16.0);
    // The photo that failed to load, if any: initials instead until it changes.
    let mut broken = use_signal(|| None::<String>);
    let photo = shown_photo(photo.as_deref(), broken.read().as_deref()).map(str::to_string);
    let look = if photo.is_some() { "ax-avatar-photo".to_string() } else { format!("ax-persona-{idx}") };
    let failed = photo.clone();
    rsx! {
        span {
            class: "ax-avatar {look} {class}",
            style: "width:{size}px;height:{size}px;font-size:{font}px",
            title: if email.is_empty() { name.clone() } else { format!("{name} <{email}>") },
            "aria-hidden": "true",
            if let Some(src) = photo {
                img { src: "{src}", alt: "", draggable: "false", onerror: move |_| broken.set(failed.clone()) }
            } else if initials.is_empty() {
                Icon { name: "person", size: (size as f64 * 0.6) as u32 }
            } else {
                "{initials}"
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatars_show_only_data_image_urls() {
        assert_eq!(photo_src(Some("data:image/jpeg;base64,/9j/4AAQ")), Some("data:image/jpeg;base64,/9j/4AAQ"));
        assert_eq!(photo_src(Some("data:image/png;base64,iVBORw0KGgo=")), Some("data:image/png;base64,iVBORw0KGgo="));
        for other in ["https://graph.microsoft.com/v1.0/me/photo/$value", "file:///home/me.png", "data:text/html,<b>x</b>", "javascript:alert(1)", " data:image/png;base64,x", ""] {
            assert_eq!(photo_src(Some(other)), None, "{other}");
        }
        assert_eq!(photo_src(None), None);
    }

    #[test]
    fn a_photo_that_fails_to_load_falls_back_to_initials_until_it_changes() {
        let (a, b) = ("data:image/jpeg;base64,/9j/AAAA", "data:image/jpeg;base64,/9j/BBBB");
        assert_eq!(shown_photo(Some(a), None), Some(a));
        assert_eq!(shown_photo(Some(a), Some(a)), None);
        assert_eq!(shown_photo(Some(b), Some(a)), Some(b));
        assert_eq!(shown_photo(Some("https://example.com/a.png"), None), None);
    }
}
