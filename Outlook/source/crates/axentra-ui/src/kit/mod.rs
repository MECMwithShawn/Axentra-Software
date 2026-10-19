//! Shared components. Props here are a contract used by every module: add
//! optional props freely, but don't rename or remove existing ones.
//!
//! Keyboard and focus behaviour that has to run in the page (roving focus,
//! focus trap, focus return, tooltips) lives in [`dom`]; components only
//! mark up their structure for it.

pub mod dom;
pub mod keys;

mod button;
mod dialog;
mod feedback;
mod form;
mod menu;
mod persona;

pub use button::*;
pub use dialog::*;
pub use feedback::*;
pub use form::*;
pub use menu::*;
pub use persona::*;

/// Convert a displayed shortcut ("Ctrl+Shift+R", "Ctrl+> or Ctrl+.") to the
/// `aria-keyshortcuts` form ("Control+Shift+R", space-separated alternatives).
pub fn keys_to_aria(display: &str) -> String {
    let key = |k: &str| match k {
        "Ctrl" => "Control".to_string(),
        "Esc" => "Escape".to_string(),
        other => other.to_string(),
    };
    display
        .split(" or ")
        .map(str::trim)
        .filter(|alt| !alt.is_empty() && !alt.contains(" then ") && !alt.contains(", "))
        .map(|alt| {
            // "Ctrl++" style chords end in a literal "+".
            let (body, plus) = alt.strip_suffix("++").map(|b| (b, true)).unwrap_or((alt, false));
            let mut parts: Vec<String> = body.split('+').map(key).collect();
            if plus {
                parts.push("+".into());
            }
            parts.join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aria_keyshortcuts_format() {
        assert_eq!(keys_to_aria("Ctrl+Shift+R"), "Control+Shift+R");
        assert_eq!(keys_to_aria("Ctrl+> or Ctrl+."), "Control+> Control+.");
        assert_eq!(keys_to_aria("Esc"), "Escape");
        assert_eq!(keys_to_aria("G then I"), "");
        assert_eq!(keys_to_aria("Delete"), "Delete");
    }
}
