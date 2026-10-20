//! Small bridge to the page: the shell installs `dom.js` once, and
//! components call these helpers (from event handlers or effects) to move
//! focus, announce text to screen readers and so on.
//!
//! Markup conventions the runtime understands:
//! * `data-ax-roving="horizontal|vertical|both|grid"` on a composite widget
//!   and `data-ax-item` on its items: arrow keys, Home/End (and
//!   `data-ax-typeahead`, `data-ax-wrap`, `data-ax-cols`) move focus, and the
//!   widget keeps a single tab stop.
//! * `aria-modal="true"`: Tab is trapped in the topmost modal; Enter presses
//!   its `data-ax-default` button unless focus is on a control that handles
//!   Enter itself (buttons, links, text areas, menus, lists...).
//! * `title` / `data-ax-tip` (+ `data-ax-tip-pos="right"`): styled tooltip
//!   after 500 ms; `aria-keyshortcuts` is appended as "(Ctrl+R)".
//! * `data-ax-region`: an F6 stop inside a module (default: each
//!   `.ax-panes > .ax-pane`); `data-ax-default-focus` picks the element F6
//!   lands on the first time.
//! * `data-ax-keytip="N"`: ribbon key tip letters.

use std::sync::atomic::{AtomicU64, Ordering};

use dioxus::prelude::*;

const RUNTIME_JS: &str = include_str!("dom.js");

/// Install the runtime. Idempotent; the shell calls it once after mount.
pub fn install() {
    run(RUNTIME_JS);
}

/// Run a script without waiting for its result.
pub fn run(js: &str) {
    let _ = document::eval(js);
}

/// JSON-quote a string for embedding in a script.
pub fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn call(f: &str, args: &str) {
    run(&format!("window.axShell && axShell.{f}({args})"));
}

/// Move focus to the first element matching `selector` (keyboard-visible ring).
pub fn focus(selector: &str) {
    call("focusSel", &js_str(selector));
}

/// Focus the first focusable descendant (or `item` match) of `selector`.
pub fn focus_first_in(selector: &str, item: &str) {
    call("focusFirstIn", &format!("{}, {}", js_str(selector), js_str(item)));
}

/// Remember the focused element and focus the first item of popup `id`.
/// Focus returns to the invoker when the popup leaves the page.
pub fn open_popup(id: &str) {
    call("openPopup", &js_str(id));
}

/// Like [`open_popup`] for dialogs: focuses `[data-ax-autofocus]` or the
/// first tabbable element of the body.
pub fn open_dialog(id: &str) {
    open_dialog_with(id, None);
}

/// [`open_dialog`] with an explicit first focus: `initial` is a selector
/// inside the dialog (falls back to the usual choice when nothing matches).
pub fn open_dialog_with(id: &str, initial: Option<&str>) {
    call("openDialog", &format!("{}, {}", js_str(id), initial.map(js_str).unwrap_or_else(|| "null".into())));
}

/// The dialog `id` closed: give focus back to the element that opened it
/// when focus was lost with the dialog.
pub fn close_dialog(id: &str) {
    call("closeDialog", &js_str(id));
}

/// Remember the focused element so it gets focus back later.
pub fn push_focus() {
    call("push", "");
}

/// Keep a popup inside the viewport.
pub fn fit(id: &str) {
    call("fit", &js_str(id));
}

/// Next / previous F6 region.
pub fn cycle_regions(dir: i32) {
    call("cycle", &dir.to_string());
}

/// Focus the F6 region matching `selector` (its remembered or default element).
pub fn enter_region(selector: &str) {
    call("enterRegion", &js_str(selector));
}

/// Speak `text` through the polite (or assertive) live region.
pub fn announce(text: &str, assertive: bool) {
    call("announce", &format!("{}, {assertive}", js_str(text)));
}

/// Scroll to the element and briefly highlight it.
pub fn flash(selector: &str) {
    call("flash", &js_str(selector));
}

/// Chords whose browser default must never run (`always`) or only outside
/// text fields (`outside`).
pub fn set_prevent(always: &[&str], outside: &[&str]) {
    let a = serde_json::to_string(always).unwrap_or_default();
    let o = serde_json::to_string(outside).unwrap_or_default();
    call("setPrevent", &format!("{a}, {o}"));
}

pub fn set_keytips(on: bool) {
    call("setKeytips", if on { "true" } else { "false" });
}

/// A per-instance DOM id such as `ax-menu-12`, stable for the component's life.
pub fn use_unique_id(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let prefix = prefix.to_string();
    use_hook(move || format!("{prefix}-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_strings_are_quoted() {
        assert_eq!(js_str("a\"b"), "\"a\\\"b\"");
        assert_eq!(js_str("#x .y"), "\"#x .y\"");
    }

    #[test]
    fn runtime_defines_the_api_the_helpers_call() {
        for f in ["focusSel", "focusFirstIn", "openPopup", "openDialog", "push", "fit", "cycle", "announce", "flash", "setPrevent", "setKeytips"] {
            assert!(RUNTIME_JS.contains(&format!("ax.{f} =")), "dom.js lacks ax.{f}");
        }
    }
}
