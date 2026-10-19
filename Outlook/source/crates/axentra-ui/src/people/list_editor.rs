//! "New contact list" / "Edit contact list" dialog with a people picker
//! (docs/spec/people-todo.md 2.9).

use axentra_core::*;
use dioxus::prelude::*;

use super::avatar::PeopleAvatar;
use super::ctx::{LIST_WARN_MEMBERS, PeopleCtx};
use super::details::open_item;
use super::dialogs::ConfirmDialog;
use super::form::valid_email;
use super::view::Item;
use crate::kit::{Dialog, Icon, IconButton, keys};
use crate::state::{AppState, store, use_app};

/// Add one address to the draft list; duplicates are ignored with a toast.
fn add_member(app: AppState, list: &mut Signal<ContactList>, member: EmailAddress) {
    let address = member.address.trim().to_string();
    if address.is_empty() {
        return;
    }
    if list.read().members.iter().any(|m| m.address.eq_ignore_ascii_case(&address)) {
        app.toast(format!("{address} is already in this list"));
        return;
    }
    // Prefer the saved contact's name for raw addresses.
    let name = if member.name.trim().is_empty() {
        store().contact_by_email(&address).ok().flatten().map(|c| c.display_name).unwrap_or_default()
    } else {
        member.name.trim().to_string()
    };
    list.write().members.push(EmailAddress::new(name, address));
}

#[component]
pub fn ListEditorDialog(id: Option<ContactListId>, seed: Vec<EmailAddress>) -> Element {
    let app = use_app();
    let ctx = use_context::<PeopleCtx>();
    let original = use_hook(|| {
        let base = id.and_then(|id| store().contact_list(id).ok().flatten()).unwrap_or_default();
        let mut l = base;
        for m in &seed {
            if !l.members.iter().any(|x| x.address.eq_ignore_ascii_case(&m.address)) {
                l.members.push(m.clone());
            }
        }
        l
    });
    let mut list = use_signal(|| original.clone());
    let mut query = use_signal(String::new);
    let mut highlight = use_signal(|| 0usize);
    let mut picker_error = use_signal(|| None::<String>);
    let mut confirm = use_signal(|| false);
    let is_new = id.is_none();
    let dirty = list() != original;
    let can_save = !list.read().name.trim().is_empty();

    let q = query();
    let suggestions: Vec<EmailAddress> = if q.trim().is_empty() {
        Vec::new()
    } else {
        let members = list.read().members.clone();
        store()
            .suggest_addresses(q.trim(), 12)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !members.iter().any(|m| m.address.eq_ignore_ascii_case(&s.address)))
            .take(8)
            .collect()
    };
    let hl = highlight().min(suggestions.len().saturating_sub(1));

    // Commit typed text: a highlighted suggestion, else raw addresses.
    let mut commit = {
        let suggestions = suggestions.clone();
        move |use_suggestion: bool| {
            let text = query.peek().trim().to_string();
            if use_suggestion && let Some(s) = suggestions.get(hl).cloned() {
                add_member(app, &mut list, s);
                query.set(String::new());
                picker_error.set(None);
                return;
            }
            if text.is_empty() {
                return;
            }
            let mut bad = Vec::new();
            for part in text.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
                if valid_email(part) {
                    add_member(app, &mut list, EmailAddress::new("", part));
                } else {
                    bad.push(part.to_string());
                }
            }
            picker_error.set((!bad.is_empty()).then(|| "Enter a valid email address".to_string()));
            query.set(bad.join(", "));
        }
    };

    let submit = move || {
        if list.read().name.trim().is_empty() {
            return;
        }
        let mut l = list.read().clone();
        let result = if l.id == 0 { store().insert_contact_list(&mut l) } else { store().update_contact_list(&l).map(|_| l.id) };
        if let Some(saved) = app.check(result) {
            app.bump_people();
            ctx.close_dialog();
            open_item(ctx, Item::List(saved));
            ctx.request_focus(true);
        }
    };
    let mut cancel = move || {
        if dirty {
            confirm.set(true);
        } else {
            ctx.close_dialog();
        }
    };

    let members = list.read().members.clone();
    let count = members.len();
    rsx! {
        Dialog {
            title: if is_new { "New contact list" } else { "Edit contact list" },
            width: 600,
            class: "ax-people-form-dialog",
            on_close: move |_| cancel(),
            actions: rsx! {
                button {
                    r#type: "button",
                    class: "ax-btn ax-primary",
                    disabled: !can_save,
                    title: if can_save { "" } else { "Enter a name for the list" },
                    onclick: move |_| submit(),
                    if is_new { "Create" } else { "Save" }
                }
                button { r#type: "button", class: "ax-btn", onclick: move |_| cancel(), "Cancel" }
            },
            div {
                class: "ax-people-form ax-people-list-editor",
                onkeydown: move |e: KeyboardEvent| match keys::chord(&e).as_str() {
                    "Ctrl+S" => {
                        e.prevent_default();
                        e.stop_propagation();
                        submit();
                    }
                    "Escape" => {
                        e.stop_propagation();
                        cancel();
                    }
                    _ => {}
                },
                label { class: "ax-people-input",
                    span { class: "ax-field-label", "List name" }
                    input {
                        class: "ax-input",
                        value: "{list.read().name}",
                        "aria-label": "List name",
                        "aria-required": "true",
                        onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                        oninput: move |e| list.write().name = e.value(),
                        onkeydown: move |e| {
                            keys::stop_typing_keys(&e);
                            if keys::chord(&e) == "Ctrl+A" { e.stop_propagation() }
                        },
                    }
                }
                div { class: "ax-people-input ax-people-picker",
                    span { class: "ax-field-label", "Add email addresses" }
                    input {
                        class: "ax-input",
                        class: if picker_error().is_some() { "ax-invalid" },
                        value: "{q}",
                        placeholder: "Enter a name or email address",
                        "aria-label": "Add email addresses",
                        role: "combobox",
                        "aria-expanded": if suggestions.is_empty() { "false" } else { "true" },
                        "aria-controls": "ax-people-picker-list",
                        "aria-autocomplete": "list",
                        oninput: {
                            let mut commit = commit.clone();
                            move |e: FormEvent| {
                            let v = e.value();
                            highlight.set(0);
                            picker_error.set(None);
                            if v.contains([',', ';']) {
                                query.set(v);
                                commit(false);
                            } else {
                                query.set(v);
                            }
                        }},
                        onkeydown: move |e: KeyboardEvent| {
                            keys::stop_typing_keys(&e);
                            match keys::chord(&e).as_str() {
                                "Ctrl+A" => e.stop_propagation(),
                                "Enter" => {
                                    e.prevent_default();
                                    commit(true);
                                }
                                "ArrowDown" => {
                                    e.prevent_default();
                                    highlight.set(hl + 1);
                                }
                                "ArrowUp" => {
                                    e.prevent_default();
                                    highlight.set(hl.saturating_sub(1));
                                }
                                "Escape" if !query.peek().is_empty() => {
                                    e.stop_propagation();
                                    query.set(String::new());
                                }
                                _ => {}
                            }
                        },
                    }
                    if let Some(err) = picker_error() { span { class: "ax-people-error", role: "alert", "{err}" } }
                    if !suggestions.is_empty() {
                        div { id: "ax-people-picker-list", class: "ax-people-suggestions", role: "listbox", "aria-label": "Suggestions",
                            for (i, s) in suggestions.iter().cloned().enumerate() {
                                div {
                                    key: "{s.address}",
                                    class: "ax-people-suggestion",
                                    class: if i == hl { "ax-active" },
                                    role: "option",
                                    "aria-selected": if i == hl { "true" } else { "false" },
                                    onmousedown: move |e| e.prevent_default(),
                                    onclick: move |_| {
                                        add_member(app, &mut list, s.clone());
                                        query.set(String::new());
                                    },
                                    PeopleAvatar { name: s.display().to_string(), color_key: s.address.to_lowercase(), size: 24 }
                                    span { class: "ax-people-mini-text",
                                        span { class: "ax-ellipsis", "{s.display()}" }
                                        span { class: "ax-people-mini-sub ax-ellipsis", "{s.address}" }
                                    }
                                }
                            }
                        }
                    }
                }
                label { class: "ax-people-input",
                    span { class: "ax-field-label", "Description" }
                    textarea {
                        class: "ax-textarea",
                        rows: "3",
                        value: "{list.read().description}",
                        "aria-label": "Description",
                        oninput: move |e| list.write().description = e.value(),
                        onkeydown: move |e| {
                            keys::stop_typing_keys(&e);
                            if keys::chord(&e) == "Ctrl+A" { e.stop_propagation() }
                        },
                    }
                }
                if count >= LIST_WARN_MEMBERS {
                    div { class: "ax-message-bar ax-warning", role: "status",
                        Icon { name: "warning" }
                        span { "Lists with more than {LIST_WARN_MEMBERS} members may not work with every mail server." }
                    }
                }
                div { class: "ax-people-members-head", "Members ({count})" }
                div { class: "ax-people-members", role: "list",
                    if members.is_empty() {
                        div { class: "ax-people-muted", "Add people by name or email address." }
                    }
                    for (i, m) in members.into_iter().enumerate() {
                        div { key: "{m.address}", class: "ax-people-member", role: "listitem",
                            PeopleAvatar { name: m.display().to_string(), color_key: m.address.to_lowercase(), size: 32 }
                            span { class: "ax-people-mini-text",
                                span { class: "ax-ellipsis", "{m.display()}" }
                                span { class: "ax-people-mini-sub ax-ellipsis", "{m.address}" }
                            }
                            IconButton {
                                icon: "dismiss",
                                title: "Remove {m.display()}",
                                small: true,
                                onclick: move |_| { list.write().members.remove(i); },
                            }
                        }
                    }
                }
            }
        }
        if confirm() {
            ConfirmDialog {
                title: "Discard changes?",
                body: "Your changes to this list won't be saved.",
                confirm_label: "Discard",
                cancel_label: "Keep editing",
                on_confirm: move |_| {
                    confirm.set(false);
                    ctx.close_dialog();
                },
                on_cancel: move |_| confirm.set(false),
            }
        }
    }
}
