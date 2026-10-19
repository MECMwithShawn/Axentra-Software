//! Header rows: From, Subject, and the one-line header of an inline reply.

use std::rc::Rc;

use axentra_core::ComposeKind;
use dioxus::prelude::*;

use super::state::Form;
use crate::kit::{Icon, IconButton, Menu, MenuButton, MenuItem, Variant, keys};
use crate::state::{AppState, store, use_app};

/// "From" with the account picker (shown with several accounts, or when
/// Options > Show From is on).
#[component]
pub fn FromRow() -> Element {
    let app = use_app();
    let mut form = use_context::<Form>();
    let _ = app.accounts_rev();
    let accounts = store().accounts().unwrap_or_default();
    let current = form.draft.read().account_id;
    let label = accounts
        .iter()
        .find(|a| a.id == current)
        .map(|a| format!("{} <{}>", a.display_name, a.email))
        .unwrap_or_else(|| "Choose an account".into());
    let items: Vec<MenuItem> = accounts
        .iter()
        .map(|a| MenuItem::new(a.id.to_string(), format!("{} <{}>", a.display_name, a.email)).checked(a.id == current))
        .collect();
    rsx! {
        div { class: "ax-compose-row",
            span { class: "ax-compose-label ax-compose-label-static", "From" }
            MenuButton {
                label,
                items,
                variant: Variant::Subtle,
                class: "ax-compose-from",
                on_select: move |id: String| {
                    if let Ok(id) = id.parse::<i64>()
                        && id != current
                    {
                        form.draft.with_mut(|d| d.account_id = id);
                        form.touch();
                    }
                },
            }
        }
    }
}

#[component]
pub fn SubjectRow() -> Element {
    let mut form = use_context::<Form>();
    let subject = form.draft.read().subject.clone();
    rsx! {
        div { class: "ax-compose-row ax-compose-subject",
            input {
                id: form.dom("subject"),
                class: "ax-compose-subject-input",
                r#type: "text",
                placeholder: "Add a subject",
                "aria-label": "Add a subject",
                maxlength: "255",
                value: "{subject}",
                oninput: move |e| {
                    form.draft.with_mut(|d| d.subject = e.value());
                    form.touch();
                },
                onkeydown: move |e: KeyboardEvent| {
                    keys::stop_typing_keys(&e);
                    if !e.is_composing() && keys::chord(&e) == "Enter" {
                        form.exec("focus_start", serde_json::Value::Null);
                    }
                },
            }
        }
    }
}

fn kind_icon(kind: ComposeKind) -> (&'static str, &'static str) {
    match kind {
        ComposeKind::Reply => ("arrow_reply", "Reply"),
        ComposeKind::ReplyAll => ("arrow_reply_all", "Reply all"),
        ComposeKind::Forward => ("arrow_forward", "Forward"),
        ComposeKind::New => ("mail", "New message"),
    }
}

/// Switch an inline reply between Reply, Reply all and Forward: recipients,
/// subject and forwarded files follow; what was written stays.
fn switch_kind(app: AppState, mut form: Form, kind: ComposeKind) {
    let (src, account, current) = {
        let d = form.draft.peek();
        (d.source_message_id, d.account_id, d.kind)
    };
    let Some(src) = src else { return };
    if kind == current {
        return;
    }
    match store().new_draft(kind, Some(account), Some(src)) {
        Ok(fresh) => {
            let source_files: Vec<(String, i64)> = store()
                .message_body(src)
                .ok()
                .flatten()
                .map(|b| b.attachments.into_iter().map(|a| (a.name, a.size)).collect())
                .unwrap_or_default();
            form.draft.with_mut(|d| {
                d.kind = kind;
                d.to = fresh.to;
                d.cc = fresh.cc;
                d.subject = fresh.subject;
                if kind == ComposeKind::Forward {
                    for a in fresh.attachments {
                        if !d.attachments.iter().any(|x| x.name == a.name && x.size == a.size) {
                            d.attachments.push(a);
                        }
                    }
                } else {
                    d.attachments.retain(|a| !source_files.iter().any(|(n, s)| *n == a.name && *s == a.size));
                }
            });
            let has_cc = !form.draft.peek().cc.is_empty();
            form.show_cc.set(has_cc);
            form.touch();
            if kind == ComposeKind::Forward {
                form.expanded.set(true);
                form.focus("to");
            }
        }
        Err(e) => {
            app.error(format!("Couldn't switch to {}: {e}", kind_icon(kind).1));
        }
    }
}

/// The inline reply's one-line header: reply type, who it goes to, and
/// "…" to show every field.
#[component]
pub fn CompactHeader(#[props(default)] trailing: Option<Element>) -> Element {
    let app = use_app();
    let mut form = use_context::<Form>();
    let mut menu = use_signal(|| None::<(f64, f64)>);
    let mut anchor = use_signal(|| None::<Rc<MountedData>>);
    let d = form.draft.read();
    let kind = d.kind;
    let (icon, label) = kind_icon(kind);
    let names = |l: &[axentra_core::EmailAddress]| l.iter().map(|a| a.display().to_string()).collect::<Vec<_>>().join("; ");
    // (label, names) for each non-empty well; To always shows.
    let lines: Vec<(&'static str, String)> = [("To:", names(&d.to)), ("Cc:", names(&d.cc)), ("Bcc:", names(&d.bcc))]
        .into_iter()
        .enumerate()
        .filter(|(i, (_, n))| *i == 0 || !n.is_empty())
        .map(|(_, l)| l)
        .collect();
    let empty = d.to.is_empty() && d.cc.is_empty() && d.bcc.is_empty();
    let can_switch = d.source_message_id.is_some();
    drop(d);
    let items = vec![
        MenuItem::new("reply", "Reply").icon("arrow_reply").checked(kind == ComposeKind::Reply),
        MenuItem::new("reply_all", "Reply all").icon("arrow_reply_all").checked(kind == ComposeKind::ReplyAll),
        MenuItem::new("forward", "Forward").icon("arrow_forward").checked(kind == ComposeKind::Forward),
    ];
    rsx! {
        div { class: "ax-compose-row ax-compose-compact",
            button {
                r#type: "button",
                class: "ax-compose-kind",
                title: if can_switch { "{label}: change how you respond" } else { "{label}" },
                "aria-label": "{label}",
                "aria-haspopup": "menu",
                "aria-expanded": if menu().is_some() { "true" } else { "false" },
                disabled: !can_switch,
                onmounted: move |e| anchor.set(Some(e.data())),
                onclick: move |_| {
                    spawn(async move {
                        let rect = match anchor() {
                            Some(m) => m.get_client_rect().await.ok(),
                            None => None,
                        };
                        menu.set(Some(rect.map(|r| (r.min_x(), r.max_y() + 4.0)).unwrap_or((0.0, 0.0))));
                    });
                },
                Icon { name: icon }
                Icon { name: "chevron_down", size: 12 }
            }
            button {
                r#type: "button",
                class: "ax-compose-summary",
                title: "Edit recipients",
                onclick: move |_| { form.expanded.set(true); form.focus("to"); },
                span { class: "ax-ellipsis",
                    for (i, (label, list)) in lines.into_iter().enumerate() {
                        span { key: "{label}", class: "ax-compose-summary-label", class: if i > 0 { "ax-next" }, "{label} " }
                        if empty { span { class: "ax-muted", "Add recipients" } } else { "{list}" }
                    }
                }
            }
            IconButton {
                icon: "more_horizontal",
                title: "Show all fields",
                onclick: move |_| { form.expanded.set(true); form.focus("to"); },
            }
            if let Some(t) = trailing { {t} }
        }
        if let Some((x, y)) = menu() {
            Menu {
                items,
                x,
                y,
                on_close: move |_| menu.set(None),
                on_select: move |id: String| {
                    let kind = match id.as_str() {
                        "reply" => ComposeKind::Reply,
                        "reply_all" => ComposeKind::ReplyAll,
                        _ => ComposeKind::Forward,
                    };
                    switch_kind(app, form, kind);
                },
            }
        }
    }
}
