//! Ribbon command ids and form-level keys, mapped to editor commands and
//! form actions.

use axentra_core::{Importance, KeyboardMode};
use chrono::Local;
use dioxus::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};

use super::actions;
use super::attachments;
use super::draw::Tool;
use super::format::{STYLES, font_stack, step_size};
use super::state::{ComposeDialog, Flyout, Form};
use crate::kit::keys;
use crate::state::{AppState, SettingsPage};

/// Run a ribbon command (`compose.*` id, optionally `compose.name:arg`).
pub fn run(app: AppState, mut form: Form, id: &str) {
    let Some(cmd) = id.strip_prefix("compose.") else { return };
    let (name, arg) = match cmd.split_once(':') {
        Some((n, a)) => (n, Some(a)),
        None => (cmd, None),
    };
    let plain = form.draft.peek().plain_text;
    match (name, arg) {
        ("bold" | "italic" | "underline" | "strike" | "sub" | "sup" | "bullets" | "numbering" | "indent" | "outdent" | "left"
        | "center" | "right" | "justify" | "clear" | "hr", None) => {
            if !plain {
                form.exec(name, Value::Null);
            }
        }
        ("ltr" | "rtl", None) => form.exec("dir", json!(name)),
        ("grow" | "shrink", None) => {
            let cur = form.fmt.peek().size;
            let cur = if cur > 0.0 { cur } else { app.settings.peek().default_font_size_pt as f32 };
            form.exec("size", json!(step_size(cur, name == "grow")));
        }
        ("font", Some(f)) => form.exec("font", json!(font_stack(f))),
        ("size", Some(s)) => {
            if let Ok(pt) = s.parse::<f32>() {
                form.exec("size", json!(pt));
            }
        }
        ("highlight", None) => form.exec("highlight", json!(form.highlight.peek().clone())),
        ("highlight", Some(v)) => {
            if v != "none" {
                form.highlight.set(v.to_string());
            }
            form.exec("highlight", json!(v));
        }
        ("color", None) => form.exec("color", json!(form.color.peek().clone())),
        ("color", Some(v)) => {
            if v != "auto" {
                form.color.set(v.to_string());
            }
            form.exec("color", json!(v));
        }
        ("palette", _) => open_flyout(form, Flyout::Palette { highlight: false }),
        ("bullet_style", Some(v)) => form.exec("bullet_style", json!(v)),
        ("number_style", Some(v)) => form.exec("number_style", json!(v)),
        ("line_spacing", Some(v)) => form.exec("line_spacing", json!(v)),
        ("style", Some(sid)) => {
            if let Some((_, _, tag)) = STYLES.iter().find(|(i, _, _)| *i == sid) {
                form.exec("block", json!(tag));
            }
        }
        ("link", _) => {
            if !plain {
                form.spawn(open_link_dialog(form));
            }
        }
        ("attach", _) => click(form, "file"),
        ("recent", Some(path)) => {
            form.spawn(attachments::add_recent(app, form, path.to_string()));
        }
        ("draw", arg) => {
            if !plain {
                let (tool, color) = arg.and_then(|a| a.split_once(':')).unwrap_or(("pen", "#000000"));
                form.dialog.set(Some(ComposeDialog::Draw { tool: Tool::parse(tool), color: color.to_string() }));
            }
        }
        ("picture", _) => click(form, "picture"),
        ("table", _) => open_flyout(form, Flyout::Table),
        ("emoji", _) => open_flyout(form, Flyout::Emoji),
        ("signature", Some("default")) => {
            let sig = app.settings.peek().signature_html.clone();
            if !sig.trim().is_empty() {
                form.exec("signature", json!(sig));
            }
        }
        ("signatures", _) => {
            let mut dialog = app.settings_dialog;
            dialog.set(Some(SettingsPage::Accounts));
        }
        ("date", _) => {
            let h24 = app.settings.peek().use_24h_time;
            let now = Local::now();
            let text = if h24 { now.format("%A, %B %-d, %Y %H:%M") } else { now.format("%A, %B %-d, %Y %-I:%M %p") };
            form.exec("text", json!(text.to_string()));
        }
        ("importance_high" | "importance_low", _) => {
            let want = if name == "importance_high" { Importance::High } else { Importance::Low };
            form.draft.with_mut(|d| d.importance = if d.importance == want { Importance::Normal } else { want });
            form.touch();
        }
        ("show_from", _) => {
            let v = *form.show_from.peek();
            form.show_from.set(!v);
            form.expanded.set(true);
        }
        ("show_bcc", _) => {
            let v = *form.show_bcc.peek();
            form.show_bcc.set(!v);
            form.expanded.set(true);
            if !v {
                form.focus("bcc");
            }
        }
        ("read_receipt", _) => {
            form.draft.with_mut(|d| d.request_read_receipt = !d.request_read_receipt);
            form.touch();
        }
        ("delivery_receipt", _) => {
            form.draft.with_mut(|d| d.request_delivery_receipt = !d.request_delivery_receipt);
            form.touch();
        }
        ("mode", Some(m)) => {
            let to_plain = m == "plain";
            if to_plain != plain {
                form.exec("plain", json!(to_plain));
                form.draft.with_mut(|d| d.plain_text = to_plain);
                form.touch();
            }
        }
        ("save", _) => {
            form.spawn(save(app, form));
        }
        ("schedule", _) => form.dialog.set(Some(ComposeDialog::Schedule)),
        ("send", _) => {
            form.spawn(actions::send(app, form));
        }
        ("discard", _) => {
            form.spawn(actions::discard(app, form));
        }
        ("popout", _) => {
            form.spawn(actions::set_popout(app, form, true));
        }
        ("popin", _) => {
            form.spawn(actions::set_popout(app, form, false));
        }
        _ => tracing::debug!("unhandled compose command {id}"),
    }
}

/// Ctrl+S / Save draft: save now and say so.
pub async fn save(app: AppState, form: Form) {
    actions::refresh_body(form).await;
    if actions::save_now(app, form).is_some() {
        app.toast("Draft saved");
    } else {
        app.error("Couldn't save draft");
    }
}

/// Programmatic click on one of the form's hidden file inputs.
pub fn click(form: Form, part: &str) {
    let id = form.dom(part);
    document::eval(&format!("const e = document.getElementById({id:?}); if (e) {{ e.value = ''; e.click(); }}"));
}

/// Open a flyout under the ribbon command that was just clicked.
pub fn open_flyout(mut form: Form, flyout: Flyout) {
    form.spawn(async move {
        let js = "const r = window.axLastCommandRect; return r ? [r.left, r.bottom] : null;";
        let pos = document::eval(js).join::<Option<(f64, f64)>>().await.ok().flatten();
        let (x, y) = pos.map(|(x, y)| (x, y + 4.0)).unwrap_or((240.0, 140.0));
        form.flyout.set(Some((flyout, x, y)));
    });
}

#[derive(Deserialize)]
struct LinkInfo {
    text: String,
    url: String,
    editing: bool,
}

/// Ctrl+K / Link: prefill from the selection or the link under the caret.
pub async fn open_link_dialog(mut form: Form) {
    let id = form.dom("body");
    let js = format!("const ed = window.axComposeEditors && window.axComposeEditors[{id:?}]; return ed ? ed.getLink() : null;");
    let info = document::eval(&js).join::<Option<LinkInfo>>().await.ok().flatten();
    let (text, url, editing) = info.map(|i| (i.text, i.url, i.editing)).unwrap_or_default();
    form.dialog.set(Some(ComposeDialog::Link { text, url, editing }));
}

/// Chords handled by the body editor itself (in its script); they must not
/// reach the message list (Ctrl+U is Underline here, not Mark as unread).
const EDITOR_CHORDS: &[&str] = &[
    "Ctrl+B", "Ctrl+I", "Ctrl+U", "Ctrl+E", "Ctrl+L", "Ctrl+R", "Ctrl+J", "Ctrl+T", "Ctrl+Shift+T", "Ctrl+Y", "Ctrl+Z",
    "Ctrl+Shift+Z", "Ctrl+Space", "Ctrl+]", "Ctrl+[", "Ctrl+Shift+L", "Ctrl+Shift+J", "Ctrl+Shift+V", "Ctrl+Shift+.",
    "Ctrl+Shift+,", "Ctrl+Shift+>", "Ctrl+Shift+<", "Ctrl+A", "Ctrl+C", "Ctrl+X", "Ctrl+V",
];

/// List and reading shortcuts that must do nothing while typing in a form.
const LIST_CHORDS: &[&str] = &["Ctrl+Q", "Ctrl+U", "Ctrl+R", "Ctrl+Shift+R", "Ctrl+F", "Ctrl+Shift+F", "Ctrl+Shift+V", "Ctrl+Y", "Ctrl+D", "Ctrl+Shift+Y", "Ctrl+Alt+F"];

/// Keys for the whole form (bubbling from its fields and buttons).
pub fn on_form_key(app: AppState, mut form: Form, e: &KeyboardEvent) {
    if e.is_composing() {
        return;
    }
    let chord = keys::chord(e);
    // A dialog, flyout or the mention list is the innermost layer: Esc
    // closes only that (keyboard spec 23.12), and while a dialog is open
    // the form's own shortcuts wait.
    let (dialog, flyout, mention) = (form.dialog.peek().is_some(), form.flyout.peek().is_some(), form.mention.peek().is_some());
    if chord == "Escape" && (dialog || flyout || mention) {
        e.stop_propagation();
        if dialog {
            form.send_opts.set(Default::default());
            form.dialog.set(None);
        } else if flyout {
            form.flyout.set(None);
        } else {
            form.mention.set(None);
            form.exec("mention_count", json!(0));
        }
        form.exec("focus", Value::Null);
        return;
    }
    if dialog {
        if LIST_CHORDS.contains(&chord.as_str()) || ["Ctrl+Enter", "Alt+S", "Ctrl+S"].contains(&chord.as_str()) {
            e.stop_propagation();
        }
        return;
    }
    match chord.as_str() {
        "Escape" => {
            e.stop_propagation();
            form.spawn(actions::close(app, form));
        }
        // "Turn off keyboard shortcuts" turns off Ctrl+Enter send too
        // (keyboard spec 1.2); Esc and text editing keep working.
        "Ctrl+Enter" | "Alt+S" if app.settings.peek().keyboard_mode != KeyboardMode::Off => {
            e.prevent_default();
            e.stop_propagation();
            if !e.is_auto_repeating() {
                form.spawn(actions::send(app, form));
            }
        }
        "Ctrl+S" => {
            e.prevent_default();
            e.stop_propagation();
            if !e.is_auto_repeating() {
                form.spawn(save(app, form));
            }
        }
        c if LIST_CHORDS.contains(&c) => e.stop_propagation(),
        _ => {}
    }
}

/// Keys in the body editor: formatting chords stay in the editor; Ctrl+K
/// opens the link dialog. Form keys bubble to [`on_form_key`].
pub fn on_body_key(form: Form, e: &KeyboardEvent) {
    keys::stop_typing_keys(e);
    let chord = keys::chord(e);
    if chord == "Ctrl+K" {
        e.prevent_default();
        e.stop_propagation();
        if !form.draft.peek().plain_text {
            form.spawn(open_link_dialog(form));
        }
    } else if EDITOR_CHORDS.contains(&chord.as_str()) {
        e.stop_propagation();
    }
}
