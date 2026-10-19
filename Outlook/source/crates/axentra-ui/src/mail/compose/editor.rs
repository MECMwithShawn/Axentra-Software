//! The message body: a `contenteditable` element that Dioxus never renders
//! children into. `editor.js` owns its content and reports back through
//! the eval channel; formatting commands go the other way.

use dioxus::prelude::*;
use serde::Deserialize;
use serde_json::json;

use super::actions;
use super::attachments;
use super::commands;
use super::format::{FormatState, font_stack};
use super::mention;
use super::state::{Body, Form};
use crate::state::use_app;

const SCRIPT: &str = include_str!("editor.js");

#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum FromEditor {
    Ready { html: String, text: String },
    Input,
    Snap { html: String, text: String },
    Fmt(FormatState),
    /// A file dropped on the form (base64; `None` when unreadable or too big).
    File {
        name: String,
        #[serde(rename = "type")]
        content_type: String,
        size: i64,
        data: Option<String>,
    },
    /// "@query" before the caret; `q: None` when the mention ended.
    Mention {
        q: Option<String>,
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
    },
    /// A key for the open mention list.
    Mkey { key: String },
    /// Focus left the form: save now (mail spec 6.7).
    Leave,
}

/// The body editor. `focus_start` puts the caret at the top once loaded
/// (replies: above the signature and the quoted message).
#[component]
pub fn BodyEditor(#[props(default)] focus_start: bool) -> Element {
    let app = use_app();
    let mut form = use_context::<Form>();
    let settings = app.settings.read();
    let style = format!("font-family: {}; font-size: {}pt", font_stack(&settings.default_font), settings.default_font_size_pt.max(1));
    drop(settings);
    let plain = form.draft.read().plain_text;

    use_hook(move || {
        spawn(async move {
            let cfg = json!({
                "id": form.dom("body"),
                "root": form.dom("root"),
                "html": form.body.peek().html,
                "plain": form.draft.peek().plain_text,
            });
            let mut eval = document::eval(&SCRIPT.replacen("/*AX_CONFIG*/null", &cfg.to_string(), 1));
            loop {
                let msg = match eval.recv::<serde_json::Value>().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let Ok(msg) = serde_json::from_value::<FromEditor>(msg) else { continue };
                let mut body = form.body;
                match msg {
                    FromEditor::Ready { html, text } => {
                        // The browser's serialisation is the baseline for
                        // "changed?", so loading alone never marks it dirty.
                        body.set(Body { html, text });
                        if focus_start {
                            form.exec("focus_start", serde_json::Value::Null);
                        }
                    }
                    FromEditor::Input => form.touch(),
                    FromEditor::Snap { html, text } => {
                        if body.peek().html != html {
                            body.set(Body { html, text });
                            form.touch();
                        } else {
                            body.write().text = text;
                        }
                    }
                    FromEditor::Fmt(f) => {
                        if *form.fmt.peek() != f {
                            form.fmt.set(f);
                        }
                    }
                    FromEditor::Mention { q, x, y } => mention::update(form, q, x, y),
                    FromEditor::Mkey { key } => mention::key(form, &key),
                    FromEditor::Leave => {
                        if form.is_dirty() && (form.draft.peek().id.is_some() || actions::has_content(form)) {
                            actions::save_now(app, form);
                        }
                    }
                    FromEditor::File { name, content_type, size, data } => {
                        if attachments::too_large(app, form, &name, size) {
                            continue;
                        }
                        match data.as_deref().and_then(attachments::base64_decode) {
                            Some(bytes) => attachments::add_bytes(app, form, &name, &content_type, &bytes),
                            None => {
                                app.error(format!("Couldn't read \"{name}\"."));
                            }
                        }
                    }
                }
            }
        })
    });

    rsx! {
        div {
            id: form.dom("body"),
            class: "ax-compose-editor",
            class: if plain { "ax-plain" },
            contenteditable: "true",
            role: "textbox",
            spellcheck: "true",
            "aria-multiline": "true",
            "aria-label": "Message body",
            style: "{style}",
            onkeydown: move |e| commands::on_body_key(form, &e),
        }
    }
}
