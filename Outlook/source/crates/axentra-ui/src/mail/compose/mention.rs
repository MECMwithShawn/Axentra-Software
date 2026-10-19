//! "@" mentions in the body (mail spec 6.3): typing "@" and a name lists
//! people; picking one inserts a `mailto:` mention and adds them to To.
//! The editor script reports what follows the "@" and forwards the list
//! keys (Up, Down, Enter, Tab, Esc) while the list is open.

use axentra_core::EmailAddress;
use dioxus::prelude::*;
use serde_json::json;

use super::recipients;
use super::state::{Field, Form, Mention};
use crate::kit::Avatar;
use crate::state::store;

/// At most this many people in the list (like recipient autocomplete).
const MAX: usize = 8;

/// People for `query`: current recipients first, then suggestions.
pub fn suggestions(form: Form, query: &str) -> Vec<EmailAddress> {
    let q = query.trim().to_lowercase();
    let d = form.draft.peek();
    let mut out: Vec<EmailAddress> = d
        .to
        .iter()
        .chain(&d.cc)
        .chain(&d.bcc)
        .filter(|a| a.address.contains('@'))
        .filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q) || a.address.to_lowercase().contains(&q))
        .cloned()
        .collect();
    drop(d);
    for (a, _) in store().recipient_suggestions(query.trim(), MAX * 2).unwrap_or_default() {
        if !out.iter().any(|x| x.address.eq_ignore_ascii_case(&a.address)) {
            out.push(a);
        }
    }
    out.truncate(MAX);
    out
}

/// The editor saw "@query" before the caret (`None`: the mention ended).
/// Tells the editor whether a list is open, so Enter still makes a new
/// line when nobody matches.
pub fn update(mut form: Form, query: Option<String>, x: f64, y: f64) {
    let count = query.as_deref().map(|q| suggestions(form, q).len()).unwrap_or(0);
    if count == 0 {
        if form.mention.peek().is_some() {
            form.mention.set(None);
        }
    } else {
        let query = query.unwrap_or_default();
        form.mention.set(Some(Mention { query, x, y, hi: 0 }));
    }
    form.exec("mention_count", json!(count));
}

/// A list key forwarded by the editor.
pub fn key(mut form: Form, key: &str) {
    let Some(m) = form.mention.peek().clone() else { return };
    let list = suggestions(form, &m.query);
    if list.is_empty() {
        close(form);
        return;
    }
    let n = list.len();
    match key {
        "ArrowDown" => form.mention.set(Some(Mention { hi: (m.hi + 1) % n, ..m })),
        "ArrowUp" => form.mention.set(Some(Mention { hi: (m.hi + n - 1) % n, ..m })),
        "Enter" | "Tab" => accept(form, list[m.hi.min(n - 1)].clone()),
        _ => close(form),
    }
}

fn close(mut form: Form) {
    form.mention.set(None);
    form.exec("mention_count", json!(0));
}

/// Insert the mention and make sure the person gets the message.
pub fn accept(mut form: Form, a: EmailAddress) {
    form.mention.set(None);
    let name = if a.name.trim().is_empty() { a.address.clone() } else { a.name.clone() };
    form.exec("mention", json!({ "name": name, "address": a.address }));
    let d = form.draft.peek();
    let present = d.to.iter().chain(&d.cc).chain(&d.bcc).any(|x| x.address.eq_ignore_ascii_case(&a.address));
    drop(d);
    if !present {
        recipients::add(form, Field::To, vec![a]);
    }
}

/// The list under the caret.
#[component]
pub fn MentionList() -> Element {
    let form = use_context::<Form>();
    let Some(m) = (form.mention)() else { return rsx! {} };
    let list = suggestions(form, &m.query);
    let hi = m.hi.min(list.len().saturating_sub(1));
    rsx! {
        div {
            class: "ax-compose-suggest ax-compose-mention",
            role: "listbox",
            "aria-label": "Mention someone",
            style: "left:{m.x}px;top:{m.y}px;transform:translate(min(0px, calc(100vw - {m.x}px - 100% - 8px)), min(0px, calc(100vh - {m.y}px - 100% - 8px)))",
            for (j, a) in list.into_iter().enumerate() {
                div {
                    key: "{a.address}",
                    class: "ax-compose-suggest-row",
                    class: if j == hi { "ax-active" },
                    role: "option",
                    "aria-selected": if j == hi { "true" } else { "false" },
                    onmousedown: {
                        let a = a.clone();
                        move |e: MouseEvent| { e.prevent_default(); e.stop_propagation(); accept(form, a.clone()); }
                    },
                    onmouseenter: {
                        let m = m.clone();
                        move |_| { let mut f = form.mention; f.set(Some(Mention { hi: j, ..m.clone() })); }
                    },
                    Avatar { name: a.name.clone(), email: a.address.clone(), size: 24 }
                    div { class: "ax-compose-suggest-text",
                        span { class: "ax-compose-suggest-name ax-ellipsis", "{a.display()}" }
                        if !a.name.trim().is_empty() {
                            span { class: "ax-compose-suggest-addr ax-ellipsis", "{a.address}" }
                        }
                    }
                }
            }
        }
    }
}
