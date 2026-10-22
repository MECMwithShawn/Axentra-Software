//! Keyboard chord normalisation for shortcuts.

use dioxus::prelude::*;

/// Normalise a key event to a chord string such as `"Ctrl+Shift+R"`,
/// `"Delete"`, `"Ctrl+Enter"`, `"ArrowDown"`, `"F6"` or `"Shift+F10"`.
/// Letters and digits come from the physical key code, so Shift+1 is
/// `"Shift+1"` (not `"!"`), regardless of layout quirks. Tab too: WebKitGTK
/// reports Shift+Tab (GTK's ISO_Left_Tab) as key "Unidentified".
pub fn chord(e: &KeyboardEvent) -> String {
    let m = e.modifiers();
    let code = format!("{:?}", e.code());
    let key = if let Some(letter) = code.strip_prefix("Key") {
        letter.to_string()
    } else if let Some(digit) = code.strip_prefix("Digit") {
        digit.to_string()
    } else if code == "Tab" {
        code
    } else {
        match e.key() {
            Key::Character(c) if c == " " => "Space".to_string(),
            Key::Character(c) => c.to_uppercase(),
            other => format!("{other}"),
        }
    };
    let mut out = String::new();
    if m.ctrl() || m.meta() {
        out.push_str("Ctrl+");
    }
    if m.alt() {
        out.push_str("Alt+");
    }
    if m.shift() {
        out.push_str("Shift+");
    }
    out.push_str(&key);
    out
}

/// The Tab key, with or without Shift (see [`chord`] for WebKitGTK's Shift+Tab).
pub fn is_tab(e: &KeyboardEvent) -> bool {
    e.key() == Key::Tab || e.code() == Code::Tab
}

/// Chords that edit text. Inside a text field they belong to the field
/// (undo, select all, word moves...), never to list or app shortcuts.
pub const TEXT_EDITING_CHORDS: &[&str] = &[
    "Ctrl+Z", "Ctrl+Y", "Ctrl+Shift+Z", "Ctrl+A", "Ctrl+C", "Ctrl+X", "Ctrl+V", "Ctrl+Shift+V",
    "Ctrl+Backspace", "Ctrl+Delete", "Ctrl+ArrowLeft", "Ctrl+ArrowRight", "Ctrl+Shift+ArrowLeft",
    "Ctrl+Shift+ArrowRight", "Ctrl+Home", "Ctrl+End", "Ctrl+Shift+Home", "Ctrl+Shift+End",
];

/// True for chords that belong to a focused text field (see
/// [`TEXT_EDITING_CHORDS`]).
pub fn is_text_editing_chord(chord: &str) -> bool {
    TEXT_EDITING_CHORDS.contains(&chord)
}

/// Call from every editable element's `onkeydown`: stops keys that would
/// trigger single-key shortcuts (Delete, letters, arrows, Enter) and text
/// editing chords (Ctrl+Z, Ctrl+A, ...) from bubbling to module/global
/// handlers, while letting other Ctrl/Alt chords, F6 and Escape through.
pub fn stop_typing_keys(e: &KeyboardEvent) {
    let m = e.modifiers();
    if m.ctrl() || m.meta() || m.alt() {
        if is_text_editing_chord(&chord(e)) {
            e.stop_propagation();
        }
        return;
    }
    if e.key() == Key::Escape || e.key() == Key::F6 {
        return;
    }
    e.stop_propagation();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_editing_chords_are_recognised() {
        assert!(is_text_editing_chord("Ctrl+Z"));
        assert!(is_text_editing_chord("Ctrl+Shift+ArrowLeft"));
        assert!(!is_text_editing_chord("Ctrl+1"));
        assert!(!is_text_editing_chord("Ctrl+Enter"));
    }
}
