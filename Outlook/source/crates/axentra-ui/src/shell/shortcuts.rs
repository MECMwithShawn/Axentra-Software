//! Keyboard shortcuts dialog (`?`; keyboard spec §13), generated from the
//! keymap table: sections start with the current module, a search box
//! filters rows, keys render as chips.

use axentra_core::KeyboardMode;
use dioxus::prelude::*;

use super::keymap::{self, GMAIL_KEYS, SHORTCUTS, Section, YAHOO_KEYS};
use super::use_shell;
use crate::kit::{Dialog, EmptyState, SearchField, dom};
use crate::settings::PageId;
use crate::state::{Module, use_app};

/// Section order: the current module's sections first.
pub fn section_order(module: Module) -> Vec<Section> {
    let first: &[Section] = match module {
        Module::Mail => &[Section::Mail, Section::Reading, Section::Compose, Section::Formatting],
        Module::Calendar => &[Section::Calendar],
        Module::People => &[Section::People],
        Module::Tasks => &[Section::Tasks],
    };
    let mut out: Vec<Section> = vec![Section::General];
    out.extend(first.iter().copied());
    let rest: Vec<Section> = Section::ALL.iter().copied().filter(|s| !out.contains(s)).collect();
    out.extend(rest);
    out
}

/// A display string as chip groups: alternatives (" or ", " / "), each a
/// sequence of chords (" then ", ", "), each chord a list of keys.
pub fn chips(display: &str) -> Vec<Vec<Vec<String>>> {
    display
        .split(" or ")
        .flat_map(|alt| alt.split(" / "))
        .map(|alt| {
            let steps: Vec<&str> = if alt.contains(" then ") { alt.split(" then ").collect() } else { alt.split(", ").collect() };
            steps
                .into_iter()
                .map(|step| {
                    let step = step.trim();
                    if step == "+" {
                        return vec!["+".to_string()];
                    }
                    let (body, plus) = step.strip_suffix("++").map(|b| (b, true)).unwrap_or((step, false));
                    let mut keys: Vec<String> = body.split('+').filter(|k| !k.is_empty()).map(str::to_string).collect();
                    if plus {
                        keys.push("+".into());
                    }
                    keys
                })
                .collect()
        })
        .collect()
}

/// Spoken form of a key list: "Control plus R".
fn spoken(display: &str) -> String {
    display.replace("Ctrl", "Control").replace('+', " plus ").replace("Esc", "Escape")
}

#[component]
pub fn ShortcutsDialog() -> Element {
    let app = use_app();
    let mut shell = use_shell();
    let mut query = use_signal(String::new);
    let mode = app.settings.read().keyboard_mode;
    let module = (app.module)();
    let q = query().to_lowercase();
    let matches = |action: &str, keys: &str| q.is_empty() || action.to_lowercase().contains(&q) || keys.to_lowercase().contains(&q);
    let sections: Vec<(Section, Vec<(&'static str, &'static str)>)> = section_order(module)
        .into_iter()
        .map(|s| {
            let rows = SHORTCUTS
                .iter()
                .filter(|r| r.section == s)
                .map(|r| (r.action, keymap::display_keys(r, mode)))
                .filter(|(a, k)| matches(a, k))
                .collect::<Vec<_>>();
            (s, rows)
        })
        .filter(|(_, rows)| !rows.is_empty())
        .collect();
    let overlay: &[(&str, &str)] = match mode {
        KeyboardMode::Gmail => GMAIL_KEYS,
        KeyboardMode::Yahoo => YAHOO_KEYS,
        _ => &[],
    };
    let overlay_rows: Vec<(&str, &str)> = overlay.iter().filter(|(k, a)| matches(a, k)).map(|(k, a)| (*a, *k)).collect();
    let count: usize = sections.iter().map(|(_, r)| r.len()).sum::<usize>() + overlay_rows.len();
    let off = mode == KeyboardMode::Off;
    let typed = query();
    use_effect(use_reactive((&count, &typed), move |(n, typed)| {
        if !typed.is_empty() {
            dom::announce(&if n == 1 { "1 shortcut found".to_string() } else { format!("{n} shortcuts found") }, false);
        }
    }));
    let close = move |_| shell.shortcuts_open.set(false);
    rsx! {
        Dialog { title: "Keyboard shortcuts", width: 560, resize_key: "shortcuts", min_width: 420, min_height: 320, class: "ax-shortcuts", on_close: close,
            div { class: "ax-shortcuts-mode",
                span { "Shortcut set: {keymap::mode_label(mode)}" }
                button {
                    r#type: "button",
                    class: "ax-link-btn",
                    onclick: move |_| {
                        shell.shortcuts_open.set(false);
                        shell.open_settings(app, PageId::Accessibility);
                    },
                    "Change"
                }
            }
            if off {
                p { class: "ax-muted ax-shortcuts-note",
                    "Shortcuts are turned off. Tab, the arrow keys, Enter, Esc, F6 and the Alt key tips still work."
                }
            }
            SearchField {
                value: query(),
                placeholder: "Search shortcuts",
                autofocus: true,
                oninput: move |v: String| query.set(v),
            }
            div { class: "ax-shortcuts-list", "aria-live": "off",
                if count == 0 {
                    EmptyState { icon: "keyboard", title: "No shortcuts found", subtitle: "Try a different word." }
                }
                if !overlay_rows.is_empty() {
                    section {
                        h3 { class: "ax-shortcuts-section", "{keymap::mode_label(mode)} keys" }
                        for (action, keys) in overlay_rows {
                            ShortcutRow { key: "{action}", action, keys }
                        }
                    }
                }
                for (section, rows) in sections {
                    section { key: "{section.label()}",
                        h3 { class: "ax-shortcuts-section", "{section.label()}" }
                        for (action, keys) in rows {
                            ShortcutRow { key: "{action}{keys}", action, keys }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ShortcutRow(action: &'static str, keys: &'static str) -> Element {
    let groups = chips(keys);
    rsx! {
        div { class: "ax-shortcut-row", "aria-label": "{action}: {spoken(keys)}", role: "group",
            span { class: "ax-shortcut-action", "{action}" }
            span { class: "ax-shortcut-keys", "aria-hidden": "true",
                for (gi, steps) in groups.into_iter().enumerate() {
                    if gi > 0 { span { class: "ax-shortcut-sep", "or" } }
                    for (si, chord) in steps.into_iter().enumerate() {
                        if si > 0 { span { class: "ax-shortcut-sep", "then" } }
                        for k in chord {
                            kbd { class: "ax-key", "{k}" }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_split_alternatives_sequences_and_chords() {
        assert_eq!(chips("Ctrl+Shift+R"), vec![vec![vec!["Ctrl".to_string(), "Shift".into(), "R".into()]]]);
        assert_eq!(chips("F6 or Ctrl+F6").len(), 2);
        assert_eq!(chips("G then I"), vec![vec![vec!["G".to_string()], vec!["I".to_string()]]]);
        assert_eq!(chips("Alt, H / Alt, V").len(), 2);
        assert_eq!(chips("Ctrl++"), vec![vec![vec!["Ctrl".to_string(), "+".into()]]]);
    }

    #[test]
    fn current_module_comes_after_general() {
        assert_eq!(section_order(Module::Calendar)[..2], [Section::General, Section::Calendar]);
        assert_eq!(section_order(Module::Mail).len(), Section::ALL.len());
    }
}
