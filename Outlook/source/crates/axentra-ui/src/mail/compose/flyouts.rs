//! Flyouts opened from the ribbon: table grid, emoji picker and the full
//! colour palette.

use dioxus::prelude::*;
use serde_json::json;

use super::emoji;
use super::format::{STANDARD_COLORS, theme_palette};
use super::state::{ComposeDialog, Flyout, Form};
use crate::kit::keys;

/// Emoji used this session, most recent first.
static RECENT_EMOJI: GlobalSignal<Vec<&'static str>> = Signal::global(Vec::new);

#[component]
pub fn ComposeFlyouts() -> Element {
    let mut form = use_context::<Form>();
    let Some((which, x, y)) = (form.flyout)() else { return rsx! {} };
    let (label, body) = match which {
        Flyout::Table => ("Insert table", rsx! { TablePicker {} }),
        Flyout::Emoji => ("Emoji", rsx! { EmojiPicker {} }),
        Flyout::Palette { highlight } => ("More colors", rsx! { Palette { highlight } }),
    };
    rsx! {
        div {
            class: "ax-menu-backdrop",
            onclick: move |e| { e.stop_propagation(); form.flyout.set(None); },
        }
        div {
            class: "ax-compose-flyout",
            role: "dialog",
            "aria-label": "{label}",
            tabindex: "-1",
            style: "left:{x}px;top:{y}px;transform:translate(min(0px, calc(100vw - {x}px - 100% - 8px)), min(0px, calc(100vh - {y}px - 100% - 8px)))",
            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape {
                    e.stop_propagation();
                    form.flyout.set(None);
                    form.exec("focus", serde_json::Value::Null);
                }
            },
            {body}
        }
    }
}

const GRID_COLS: usize = 10;
const GRID_ROWS: usize = 8;

#[component]
fn TablePicker() -> Element {
    let mut form = use_context::<Form>();
    let mut at = use_signal(|| (0usize, 0usize));
    let (r, c) = at();
    let caption = if r == 0 { "Insert table".to_string() } else { format!("{c} x {r} table") };
    rsx! {
        div { class: "ax-compose-flyout-title", "{caption}" }
        div {
            class: "ax-compose-tablegrid",
            role: "grid",
            "aria-label": "Table size",
            onmouseleave: move |_| at.set((0, 0)),
            for row in 1..=GRID_ROWS {
                for col in 1..=GRID_COLS {
                    button {
                        key: "{row}-{col}",
                        r#type: "button",
                        class: "ax-compose-tablecell",
                        class: if row <= r && col <= c { "ax-on" },
                        "aria-label": "{col} by {row} table",
                        onmouseenter: move |_| at.set((row, col)),
                        onfocus: move |_| at.set((row, col)),
                        onclick: move |_| {
                            form.flyout.set(None);
                            form.exec("table", json!({ "rows": row, "cols": col }));
                        },
                    }
                }
            }
        }
        div { class: "ax-menu-divider" }
        button {
            r#type: "button",
            class: "ax-menu-item",
            onclick: move |_| { form.flyout.set(None); form.dialog.set(Some(ComposeDialog::TableSize)); },
            span { class: "ax-menu-label", "Insert table…" }
        }
    }
}

#[component]
fn EmojiPicker() -> Element {
    let form = use_context::<Form>();
    let mut query = use_signal(String::new);
    let mut category = use_signal(|| emoji::CATEGORIES[0].0);
    let q = query().trim().to_string();
    let recent = RECENT_EMOJI.read().clone();
    let shown: Vec<(&'static str, &'static str)> = if q.is_empty() {
        emoji::CATEGORIES.iter().find(|c| c.0 == category()).map(|c| c.2.to_vec()).unwrap_or_default()
    } else {
        emoji::search(&q)
    };
    let pick = move |e: &'static str| {
        form.exec("text", json!(e));
        let mut r = RECENT_EMOJI.write();
        r.retain(|x| *x != e);
        r.insert(0, e);
        r.truncate(16);
    };
    rsx! {
        div { class: "ax-compose-emoji",
            input {
                class: "ax-input",
                r#type: "search",
                placeholder: "Search emoji",
                "aria-label": "Search emoji",
                value: "{query}",
                oninput: move |e| query.set(e.value()),
                onkeydown: move |e: KeyboardEvent| keys::stop_typing_keys(&e),
            }
            if q.is_empty() {
                div { class: "ax-compose-emoji-tabs", role: "tablist", "aria-label": "Emoji categories",
                    for (id, label, list) in emoji::CATEGORIES.iter() {
                        button {
                            key: "{id}",
                            r#type: "button",
                            role: "tab",
                            class: "ax-compose-emoji-tab",
                            class: if category() == *id { "ax-selected" },
                            "aria-selected": if category() == *id { "true" } else { "false" },
                            title: "{label}",
                            "aria-label": "{label}",
                            onclick: move |_| category.set(id),
                            "{list[0].0}"
                        }
                    }
                }
                if !recent.is_empty() {
                    div { class: "ax-compose-emoji-heading", "Recently used" }
                    div { class: "ax-compose-emoji-grid",
                        for e in recent {
                            button { key: "r{e}", r#type: "button", class: "ax-compose-emoji-btn", title: "{e}", onclick: move |_| pick(e), "{e}" }
                        }
                    }
                }
                div { class: "ax-compose-emoji-heading",
                    {emoji::CATEGORIES.iter().find(|c| c.0 == category()).map(|c| c.1).unwrap_or_default()}
                }
            }
            div { class: "ax-compose-emoji-grid ax-compose-emoji-scroll", role: "grid",
                if shown.is_empty() {
                    div { class: "ax-compose-emoji-empty", "No emoji found" }
                }
                for (e, name) in shown {
                    button { key: "{e}", r#type: "button", class: "ax-compose-emoji-btn", title: "{name}", "aria-label": "{name}", onclick: move |_| pick(e), "{e}" }
                }
            }
        }
    }
}

/// "More colors…": the 10 x 6 theme grid, standard colours and a hex box.
#[component]
fn Palette(highlight: bool) -> Element {
    let mut form = use_context::<Form>();
    let mut custom = use_signal(String::new);
    let palette = use_hook(theme_palette);
    let mut apply = move |hex: String| {
        form.flyout.set(None);
        if highlight {
            form.highlight.set(hex.clone());
            form.exec("highlight", json!(hex));
        } else {
            form.color.set(hex.clone());
            form.exec("color", json!(hex));
        }
    };
    let typed = custom().trim().trim_start_matches('#').to_uppercase();
    let valid = typed.len() == 6 && typed.chars().all(|c| c.is_ascii_hexdigit());
    let hex = format!("#{typed}");
    let hex_key = hex.clone();
    let hex_btn = hex.clone();
    rsx! {
        div { class: "ax-compose-flyout-title", "Theme colors" }
        div { class: "ax-compose-palette", role: "grid", "aria-label": "Theme colors",
            for (hex, name) in palette {
                button {
                    key: "{hex}-{name}",
                    r#type: "button",
                    class: "ax-compose-swatch",
                    style: "background:{hex}",
                    title: "{name}",
                    "aria-label": "{name}",
                    onclick: move |_| apply(hex.clone()),
                }
            }
        }
        div { class: "ax-compose-flyout-title", "Standard colors" }
        div { class: "ax-compose-palette",
            for (hex, name) in STANDARD_COLORS.iter() {
                button {
                    key: "s{hex}",
                    r#type: "button",
                    class: "ax-compose-swatch",
                    style: "background:{hex}",
                    title: "{name}",
                    "aria-label": "{name}",
                    onclick: move |_| apply(hex.to_string()),
                }
            }
        }
        div { class: "ax-compose-flyout-title", "Custom color" }
        div { class: "ax-compose-custom-color",
            span { class: "ax-compose-swatch ax-static", style: if valid { "background:{hex}" } else { "" } }
            input {
                class: "ax-input",
                r#type: "text",
                placeholder: "#RRGGBB",
                "aria-label": "Hex color",
                maxlength: "7",
                value: "{custom}",
                oninput: move |e| custom.set(e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    keys::stop_typing_keys(&e);
                    if keys::chord(&e) == "Enter" && valid {
                        apply(hex_key.clone());
                    }
                },
            }
            button {
                r#type: "button",
                class: "ax-btn ax-small",
                disabled: !valid,
                onclick: move |_| apply(hex_btn.clone()),
                "Apply"
            }
        }
    }
}
