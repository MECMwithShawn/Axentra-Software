//! To / Cc / Bcc wells: recipient chips, typing and pasting addresses,
//! and the autocomplete list (mail spec 6.3, keyboard spec 5.3).

use std::time::Duration;

use axentra_core::EmailAddress;
use dioxus::prelude::*;

use super::address;
use super::state::{ComposeDialog, Field, Form};
use crate::kit::{Avatar, Icon, Menu, MenuItem, keys};
use crate::state::{NavRequest, store, use_app};

const NO_RECIPIENTS: &str = "Add at least one recipient.";
const INVALID: &str = "Fix or remove the highlighted addresses.";

/// Add recipients to a well (skipping ones already there).
pub fn add(mut form: Form, field: Field, entries: Vec<EmailAddress>) -> bool {
    let mut added = false;
    let mut names = Vec::new();
    form.draft.with_mut(|d| {
        let list = field.list_mut(d);
        for e in entries {
            if !list.iter().any(|x| x.address.trim().eq_ignore_ascii_case(e.address.trim())) {
                names.push(e.display().to_string());
                list.push(e);
                added = true;
            }
        }
    });
    if added {
        form.touch();
        refresh_error(form);
        form.announce(format!("Added {}", names.join(", ")));
    }
    added
}

/// Clear the inline error once its cause is fixed.
fn refresh_error(mut form: Form) {
    let err = form.error.peek().clone();
    let d = form.draft.peek();
    let all = d.to.iter().chain(&d.cc).chain(&d.bcc);
    let fixed = match err.as_deref() {
        Some(NO_RECIPIENTS) => !(d.to.is_empty() && d.cc.is_empty() && d.bcc.is_empty()),
        Some(INVALID) => all.clone().all(|a| address::is_valid(&a.address)),
        _ => false,
    };
    drop(d);
    if fixed {
        form.error.set(None);
    }
}

/// A typed entry becomes a recipient: complete addresses get a name from
/// the address book; a name resolves when it matches one person, or names
/// one exactly (Outlook's "check names"); anything else stays as typed and
/// shows as an invalid chip to fix.
fn resolve(e: EmailAddress) -> EmailAddress {
    let typed = e.address.trim().to_lowercase();
    let found = store().recipient_suggestions(&typed, 8).unwrap_or_default();
    if address::is_valid(&e.address) {
        if e.name.trim().is_empty()
            && let Some((a, _)) = found.into_iter().find(|(a, _)| a.address.trim().eq_ignore_ascii_case(&typed))
        {
            return a;
        }
        return e;
    }
    pick_match(&typed, found.into_iter().map(|(a, _)| a).collect()).unwrap_or(e)
}

/// The person a typed name means: the only match, or the one whose name
/// or mailbox name is exactly what was typed.
fn pick_match(typed: &str, found: Vec<EmailAddress>) -> Option<EmailAddress> {
    if found.len() == 1 {
        return found.into_iter().next();
    }
    found.into_iter().find(|a| {
        a.name.trim().to_lowercase() == typed || a.address.split('@').next().is_some_and(|local| local.eq_ignore_ascii_case(typed))
    })
}

fn commit_text(form: Form, field: Field, text: &str) {
    let (entries, _) = address::parse_list(text, true);
    add(form, field, entries.into_iter().map(resolve).collect());
}

/// Case-insensitive match of `q` in `s`, split for highlighting.
fn split_match(s: &str, q: &str) -> Option<(String, String, String)> {
    let q = q.trim();
    if q.is_empty() {
        return None;
    }
    let ls = s.to_lowercase();
    let lq = q.to_lowercase();
    if ls.len() != s.len() {
        return None;
    }
    let i = ls.find(&lq)?;
    let j = i + lq.len();
    if !s.is_char_boundary(i) || !s.is_char_boundary(j) {
        return None;
    }
    Some((s[..i].to_string(), s[i..j].to_string(), s[j..].to_string()))
}

#[component]
fn Highlighted(text: String, query: String) -> Element {
    match split_match(&text, &query) {
        Some((a, b, c)) => rsx! { "{a}" strong { "{b}" } "{c}" },
        None => rsx! { "{text}" },
    }
}

/// A recipient row: label button (opens the address book), the well, and
/// for the To row the Cc / Bcc toggles plus any `trailing` buttons.
#[component]
pub fn RecipientRow(field: Field, #[props(default)] trailing: Option<Element>) -> Element {
    let mut form = use_context::<Form>();
    let show_cc = (form.show_cc)();
    let show_bcc = (form.show_bcc)();
    rsx! {
        div { class: "ax-compose-row ax-compose-recipients",
            button {
                r#type: "button",
                class: "ax-compose-label",
                title: "Add {field.label()} recipients from your contacts",
                onclick: move |_| form.dialog.set(Some(ComposeDialog::AddressBook(field))),
                "{field.label()}"
            }
            RecipientWell { field }
            if field == Field::To {
                div { class: "ax-compose-row-tail",
                    if !show_cc {
                        button {
                            r#type: "button",
                            class: "ax-compose-toggle",
                            title: "Show the Cc line",
                            onclick: move |_| { form.show_cc.set(true); form.focus("cc"); },
                            "Cc"
                        }
                    }
                    if !show_bcc {
                        button {
                            r#type: "button",
                            class: "ax-compose-toggle",
                            title: "Show the Bcc line",
                            onclick: move |_| { form.show_bcc.set(true); form.focus("bcc"); },
                            "Bcc"
                        }
                    }
                    if let Some(t) = trailing { {t} }
                }
            }
        }
        if field == Field::To {
            if let Some(err) = (form.error)() {
                div { class: "ax-compose-error", role: "alert",
                    Icon { name: "error_circle", size: 16 }
                    span { "{err}" }
                }
            }
        }
    }
}

#[component]
fn RecipientWell(field: Field) -> Element {
    let app = use_app();
    let form = use_context::<Form>();
    let mut text = use_signal(String::new);
    let mut open = use_signal(|| false);
    let mut hi = use_signal(|| 0usize);
    // The selected chip lives on the form, so Send can point at an invalid one.
    let mut chip = form.chip;
    let sel = move || match chip() {
        Some((f, i)) if f == field => Some(i),
        _ => None,
    };
    let mut select = move |i: Option<usize>| {
        let mine = matches!(*chip.peek(), Some((f, _)) if f == field);
        if i.is_some() || mine {
            chip.set(i.map(|i| (field, i)));
        }
    };
    let mut menu = use_signal(|| None::<(usize, f64, f64)>);
    let mut hover = use_signal(|| None::<usize>);
    let mut hover_gen = use_signal(|| 0u64);
    let mut forgot = use_signal(|| 0u64);

    let list = field.list(&form.draft.read()).clone();
    let query = text().trim().to_string();
    let suggestions: Vec<(EmailAddress, bool)> = if open() && !query.is_empty() {
        let _ = forgot();
        let _ = app.mail_rev();
        store()
            .recipient_suggestions(&query, 16)
            .unwrap_or_default()
            .into_iter()
            .filter(|(a, _)| !list.iter().any(|l| l.address.eq_ignore_ascii_case(&a.address)))
            .take(8)
            .collect()
    } else {
        Vec::new()
    };
    let showing = !suggestions.is_empty();
    let hi_now = hi().min(suggestions.len().saturating_sub(1));
    let input_id = form.dom(field.key());
    let list_id = form.dom(&format!("{}-suggest", field.key()));
    let count = list.len();

    let remove = move |i: usize| {
        let mut f = form;
        let mut removed = false;
        f.draft.with_mut(|d| {
            let l = field.list_mut(d);
            if i < l.len() {
                l.remove(i);
                removed = true;
            }
        });
        if removed {
            f.touch();
            refresh_error(f);
        }
    };
    let mut edit = move |i: usize| {
        let Some(a) = field.list(&form.draft.peek()).get(i).cloned() else { return };
        remove(i);
        text.set(address::edit_text(&a));
        select(None);
        form.focus(field.key());
    };
    let mut accept = move |a: EmailAddress| {
        add(form, field, vec![a]);
        text.set(String::new());
        open.set(false);
        hi.set(0);
        form.focus(field.key());
    };
    let picks = suggestions.clone();

    rsx! {
        div {
            class: "ax-compose-well",
            onclick: move |_| form.focus(field.key()),
            div { class: "ax-compose-chips", role: "list", "aria-label": "{field.label()} recipients",
                for (i, a) in list.iter().cloned().enumerate() {
                    {
                        let valid = address::is_valid(&a.address);
                        let name = a.display().to_string();
                        let label = match (valid, a.name.trim().is_empty()) {
                            (false, _) => format!("{name}, this address isn't valid, press Delete to remove"),
                            (true, true) => format!("{name}, press Delete to remove"),
                            (true, false) => format!("{name}, {}, press Delete to remove", a.address),
                        };
                        let tip = if valid { a.address.clone() } else { "This address isn't valid.".to_string() };
                        rsx! {
                            div {
                                key: "{i}-{a.address}",
                                class: "ax-compose-chip",
                                class: if !valid { "ax-invalid" },
                                class: if sel() == Some(i) { "ax-selected" },
                                role: "listitem",
                                "aria-label": "{label}",
                                title: "{tip}",
                                onmousedown: move |e| { e.stop_propagation(); select(Some(i)); },
                                onclick: move |e| { e.stop_propagation(); form.focus(field.key()); },
                                ondoubleclick: move |e| { e.stop_propagation(); edit(i); },
                                oncontextmenu: move |e: MouseEvent| {
                                    e.prevent_default();
                                    e.stop_propagation();
                                    let p = e.client_coordinates();
                                    select(Some(i));
                                    menu.set(Some((i, p.x, p.y)));
                                },
                                onmouseenter: move |_| {
                                    let g = hover_gen() + 1;
                                    hover_gen.set(g);
                                    spawn(async move {
                                        tokio::time::sleep(Duration::from_millis(600)).await;
                                        if hover_gen() == g {
                                            hover.set(Some(i));
                                        }
                                    });
                                },
                                onmouseleave: move |_| {
                                    hover_gen += 1;
                                    hover.set(None);
                                },
                                if valid {
                                    Avatar { name: a.name.clone(), email: a.address.clone(), size: 16 }
                                } else {
                                    Icon { name: "error_circle", size: 16 }
                                }
                                span { class: "ax-compose-chip-name", "{name}" }
                                button {
                                    r#type: "button",
                                    class: "ax-compose-chip-remove",
                                    tabindex: "-1",
                                    title: "Remove {name}",
                                    "aria-label": "Remove {name}",
                                    onclick: move |e| { e.stop_propagation(); remove(i); select(None); },
                                    Icon { name: "dismiss", size: 12 }
                                }
                                if hover() == Some(i) && menu().is_none() {
                                    HoverCard { addr: a.clone(), valid }
                                }
                            }
                        }
                    }
                }
                input {
                    id: "{input_id}",
                    class: "ax-compose-input",
                    r#type: "text",
                    role: "combobox",
                    autocomplete: "off",
                    spellcheck: "false",
                    "aria-label": "{field.label()}",
                    "aria-autocomplete": "list",
                    "aria-expanded": if showing { "true" } else { "false" },
                    "aria-controls": "{list_id}",
                    value: "{text}",
                    oninput: move |e| {
                        let v = e.value();
                        select(None);
                        hi.set(0);
                        let prev = text.peek().clone();
                        let (entries, rest) = address::split_typed(&prev, &v);
                        if !entries.is_empty() {
                            add(form, field, entries.into_iter().map(resolve).collect());
                            text.set(rest.clone());
                        } else {
                            text.set(v);
                        }
                        open.set(!text.peek().trim().is_empty());
                    },
                    onfocus: move |_| open.set(!text.peek().trim().is_empty()),
                    onblur: move |_| {
                        // Leaving the well commits what was typed (mail spec 6.3).
                        let t = text.peek().clone();
                        if !t.trim().is_empty() {
                            commit_text(form, field, &t);
                            text.set(String::new());
                        }
                        open.set(false);
                    },
                    onkeydown: move |e: KeyboardEvent| {
                        keys::stop_typing_keys(&e);
                        if e.is_composing() {
                            // Enter picks an IME candidate here, not a recipient.
                            return;
                        }
                        let empty = text.peek().is_empty();
                        let chord = keys::chord(&e);
                        match e.key() {
                            Key::ArrowDown if showing => {
                                e.prevent_default();
                                hi.set((hi_now + 1) % picks.len());
                            }
                            Key::ArrowUp if showing => {
                                e.prevent_default();
                                hi.set((hi_now + picks.len() - 1) % picks.len());
                            }
                            Key::Enter if chord == "Enter" => {
                                if showing {
                                    accept(picks[hi_now].0.clone());
                                } else if let Some(i) = sel() {
                                    edit(i);
                                } else if !empty {
                                    let t = text.peek().clone();
                                    commit_text(form, field, &t);
                                    text.set(String::new());
                                    open.set(false);
                                }
                            }
                            Key::Tab if chord == "Tab" => {
                                if showing {
                                    e.prevent_default();
                                    accept(picks[hi_now].0.clone());
                                } else if !empty {
                                    let t = text.peek().clone();
                                    commit_text(form, field, &t);
                                    text.set(String::new());
                                }
                            }
                            Key::Escape => {
                                if showing {
                                    e.stop_propagation();
                                    open.set(false);
                                } else if sel().is_some() {
                                    e.stop_propagation();
                                    select(None);
                                }
                            }
                            Key::Backspace if empty && count > 0 => match sel() {
                                Some(i) => {
                                    remove(i);
                                    select(i.checked_sub(1).filter(|_| count > 1));
                                }
                                None => select(Some(count - 1)),
                            },
                            Key::Delete if sel().is_some() => {
                                if let Some(i) = sel() {
                                    remove(i);
                                    select((i + 1 < count).then_some(i));
                                }
                            }
                            Key::ArrowLeft if empty && count > 0 => {
                                select(Some(match sel() {
                                    Some(i) => i.saturating_sub(1),
                                    None => count - 1,
                                }));
                            }
                            Key::ArrowRight if sel().is_some() => {
                                select(sel().and_then(|i| (i + 1 < count).then_some(i + 1)));
                            }
                            Key::Home if empty && count > 0 => select(Some(0)),
                            Key::End if sel().is_some() => select(None),
                            _ if chord == "Ctrl+K" => {
                                // Check names: resolve what was typed.
                                e.prevent_default();
                                e.stop_propagation();
                                if !empty {
                                    let t = text.peek().clone();
                                    commit_text(form, field, &t);
                                    text.set(String::new());
                                    open.set(false);
                                }
                            }
                            _ => {}
                        }
                    },
                }
            }
            if showing {
                div { id: "{list_id}", class: "ax-compose-suggest", role: "listbox", "aria-label": "Suggested contacts",
                    for (j, (a, recent)) in suggestions.iter().cloned().enumerate() {
                        div {
                            key: "{a.address}",
                            class: "ax-compose-suggest-row",
                            class: if j == hi_now { "ax-active" },
                            role: "option",
                            "aria-selected": if j == hi_now { "true" } else { "false" },
                            onmousedown: {
                                let a = a.clone();
                                move |e: MouseEvent| { e.stop_propagation(); accept(a.clone()); }
                            },
                            onmouseenter: move |_| hi.set(j),
                            Avatar { name: a.name.clone(), email: a.address.clone(), size: 24 }
                            div { class: "ax-compose-suggest-text",
                                span { class: "ax-compose-suggest-name ax-ellipsis",
                                    Highlighted { text: a.display().to_string(), query: query.clone() }
                                }
                                if !a.name.trim().is_empty() {
                                    span { class: "ax-compose-suggest-addr ax-ellipsis",
                                        Highlighted { text: a.address.clone(), query: query.clone() }
                                    }
                                }
                            }
                            if recent {
                                button {
                                    r#type: "button",
                                    class: "ax-compose-suggest-remove",
                                    title: "Remove from suggestions",
                                    "aria-label": "Remove {a.display()} from suggestions",
                                    onmousedown: {
                                        let addr = a.address.clone();
                                        move |e: MouseEvent| {
                                            e.stop_propagation();
                                            if let Err(err) = store().forget_recent_address(&addr) {
                                                app.error(format!("Couldn't remove the suggestion: {err}"));
                                            }
                                            forgot += 1;
                                        }
                                    },
                                    Icon { name: "dismiss", size: 12 }
                                }
                            }
                        }
                    }
                }
            }
            if let Some((i, x, y)) = menu() {
                ChipMenu { field, index: i, x, y, on_close: move |_| menu.set(None), on_edit: edit, on_remove: move |i| { remove(i); select(None); } }
            }
        }
    }
}

/// Contact details shown after hovering a chip for 600 ms.
#[component]
fn HoverCard(addr: EmailAddress, valid: bool) -> Element {
    let contact = if valid { store().contact_by_email(&addr.address).ok().flatten() } else { None };
    let subtitle = contact
        .as_ref()
        .map(|c| [c.job_title.as_str(), c.company.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    rsx! {
        div { class: "ax-compose-hovercard", role: "tooltip",
            Avatar { name: addr.name.clone(), email: addr.address.clone(), size: 40 }
            div { class: "ax-compose-hovercard-text",
                span { class: "ax-semibold ax-ellipsis", "{addr.display()}" }
                if !subtitle.is_empty() { span { class: "ax-muted ax-ellipsis", "{subtitle}" } }
                if valid {
                    span { class: "ax-ellipsis ax-selectable", "{addr.address}" }
                } else {
                    span { class: "ax-compose-invalid-text", "This address isn't valid." }
                }
            }
        }
    }
}

#[component]
fn ChipMenu(field: Field, index: usize, x: f64, y: f64, on_close: EventHandler<()>, on_edit: EventHandler<usize>, on_remove: EventHandler<usize>) -> Element {
    let app = use_app();
    let form = use_context::<Form>();
    let addr = field.list(&form.draft.peek()).get(index).cloned().unwrap_or_default();
    let contact = store().contact_by_email(&addr.address).ok().flatten();
    let items = vec![
        MenuItem::new("copy", "Copy").icon("copy"),
        MenuItem::new("edit", "Edit").icon("edit"),
        MenuItem::new("remove", "Remove").icon("dismiss"),
        MenuItem::divider(),
        MenuItem::new("card", "Open contact card").icon("contact_card").disabled(contact.is_none()),
    ];
    rsx! {
        Menu {
            items,
            x,
            y,
            on_close: move |_| on_close.call(()),
            on_select: move |id: String| match id.as_str() {
                "copy" => copy_text(&address::edit_text(&addr)),
                "edit" => on_edit.call(index),
                "remove" => on_remove.call(index),
                "card" => {
                    if let Some(c) = &contact {
                        app.navigate(NavRequest::Contact(c.id));
                    }
                }
                _ => {}
            },
        }
    }
}

/// Put text on the clipboard (async API, with the selection fallback).
pub fn copy_text(text: &str) {
    let js = format!(
        "const t = {}; \
         if (navigator.clipboard && navigator.clipboard.writeText) {{ navigator.clipboard.writeText(t).catch(() => {{}}); }} \
         else {{ const a = document.createElement('textarea'); a.value = t; document.body.appendChild(a); a.select(); document.execCommand('copy'); a.remove(); }}",
        serde_json::to_string(text).unwrap_or_default()
    );
    document::eval(&js);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_names_resolve_only_when_unambiguous() {
        let priya = EmailAddress::new("Priya Raman", "priya.raman@northwind.example");
        let hannah = EmailAddress::new("Hannah Okafor", "hannah.okafor@axentra.example");
        assert_eq!(pick_match("pri", vec![priya.clone()]), Some(priya.clone()));
        assert_eq!(pick_match("x", vec![hannah.clone(), priya.clone()]), None);
        assert_eq!(pick_match("hannah okafor", vec![priya.clone(), hannah.clone()]), Some(hannah.clone()));
        assert_eq!(pick_match("priya.raman", vec![hannah, priya.clone()]), Some(priya));
        assert_eq!(pick_match("nobody", vec![]), None);
    }

    #[test]
    fn match_highlighting() {
        assert_eq!(split_match("Priya Raman", "ram"), Some(("Priya ".into(), "Ram".into(), "an".into())));
        assert_eq!(split_match("Priya", "x"), None);
        assert_eq!(split_match("Priya", " "), None);
    }
}
