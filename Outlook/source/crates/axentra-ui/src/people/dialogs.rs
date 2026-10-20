//! Confirmations (2.14), import / export (2.13), new category and the
//! People keyboard shortcuts list.

use axentra_core::*;
use axentra_core::Store;
use dioxus::prelude::*;

use super::ctx::{PeopleCtx, PeopleDialog};
use super::form::NewContactDialog;
use super::io;
use super::list_editor::ListEditorDialog;
use super::view::{Item, Node};
use crate::kit::{Dialog, Icon, keys};
use crate::state::{SettingsPage, UndoAction, store, use_app};

/// Renders whichever dialog the module has open.
#[component]
pub fn PeopleDialogs() -> Element {
    let ctx = use_context::<PeopleCtx>();
    let Some(dialog) = ctx.dialog.read().clone() else { return rsx! {} };
    match dialog {
        PeopleDialog::NewContact(c) => rsx! { NewContactDialog { initial: *c } },
        PeopleDialog::EditList(id, seed) => rsx! { ListEditorDialog { id, seed } },
        PeopleDialog::ConfirmDelete { contacts, lists } => rsx! { DeleteConfirm { contacts, lists } },
        PeopleDialog::ConfirmDeleteFolder(name) => rsx! { DeleteFolderConfirm { name } },
        PeopleDialog::NewCategory(ids) => rsx! { NewCategoryDialog { ids } },
        PeopleDialog::Import => rsx! { ImportDialog {} },
        PeopleDialog::Export => rsx! { ExportDialog {} },
        PeopleDialog::Shortcuts => rsx! { ShortcutsDialog {} },
        PeopleDialog::DiscardEdit(next) => rsx! { DiscardEditConfirm { next } },
    }
}

/// A destructive or blocking confirmation (`role="alertdialog"`). Focus
/// starts on Cancel; Esc cancels; Enter activates the focused button.
#[component]
pub fn ConfirmDialog(
    #[props(into)] title: String,
    #[props(into)] body: String,
    #[props(into)] confirm_label: String,
    #[props(into, default = "Cancel".to_string())] cancel_label: String,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
) -> Element {
    rsx! {
        div {
            class: "ax-dialog-overlay ax-people-confirm-overlay",
            onclick: move |e| {
                e.stop_propagation();
                on_cancel.call(());
            },
            div {
                class: "ax-dialog ax-people-confirm",
                role: "alertdialog",
                "aria-modal": "true",
                "aria-labelledby": "ax-people-confirm-title",
                "aria-describedby": "ax-people-confirm-body",
                style: "width:448px",
                onclick: move |e| e.stop_propagation(),
                onkeydown: move |e: KeyboardEvent| {
                    e.stop_propagation();
                    if e.key() == Key::Escape {
                        on_cancel.call(());
                    }
                },
                div { id: "ax-people-confirm-title", class: "ax-dialog-title", "{title}" }
                div { id: "ax-people-confirm-body", class: "ax-dialog-body", "{body}" }
                div { class: "ax-dialog-actions",
                    button { r#type: "button", class: "ax-btn ax-primary", onclick: move |_| on_confirm.call(()), "{confirm_label}" }
                    button {
                        r#type: "button",
                        class: "ax-btn",
                        onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                        onclick: move |_| on_cancel.call(()),
                        "{cancel_label}"
                    }
                }
            }
        }
    }
}

/// Dialog title and body for deleting contacts and lists (2.14).
pub fn delete_texts(contacts: &[Contact], lists: &[ContactList]) -> (String, String) {
    match (contacts.len(), lists.len()) {
        (1, 0) => ("Delete contact?".into(), format!("{} will be deleted from your contacts.", contacts[0].display_name)),
        (n, 0) => (format!("Delete {n} contacts?"), "These contacts will be deleted from your contacts.".into()),
        (0, 1) => ("Delete contact list?".into(), "The people in this list won't be deleted, only the list.".into()),
        (0, n) => (format!("Delete {n} contact lists?"), "The people in these lists won't be deleted, only the lists.".into()),
        (c, l) => (format!("Delete {} items?", c + l), "These contacts and contact lists will be deleted. People in the lists stay in your contacts.".into()),
    }
}

#[component]
fn DeleteConfirm(contacts: Vec<ContactId>, lists: Vec<ContactListId>) -> Element {
    let app = use_app();
    let ctx = use_context::<PeopleCtx>();
    let c = store().contacts_by_ids(&contacts).unwrap_or_default();
    let l: Vec<ContactList> = lists.iter().filter_map(|id| store().contact_list(*id).ok().flatten()).collect();
    let (title, body) = delete_texts(&c, &l);
    rsx! {
        ConfirmDialog {
            title,
            body,
            confirm_label: "Delete",
            on_confirm: move |_| {
                ctx.close_dialog();
                super::actions::delete(ctx, app, contacts.clone(), lists.clone());
            },
            on_cancel: move |_| {
                ctx.close_dialog();
                ctx.request_focus(true);
            },
        }
    }
}

#[component]
fn DeleteFolderConfirm(name: String) -> Element {
    let app = use_app();
    let mut ctx = use_context::<PeopleCtx>();
    let n = store().contacts_in_folder_count(&name).unwrap_or(0);
    let body = format!("{name} and the {n} contact{} in it will be deleted.", if n == 1 { "" } else { "s" });
    rsx! {
        ConfirmDialog {
            title: "Delete folder?",
            body,
            confirm_label: "Delete",
            on_confirm: move |_| {
                ctx.close_dialog();
                if let Some(token) = app.check(store().delete_contact_folder(&name)) {
                    if *ctx.node.peek() == Node::Folder(name.clone()) {
                        ctx.set_node(Node::Folder(String::new()));
                    }
                    ctx.editing.set(None);
                    app.toast_with_undo("Folder deleted", UndoAction::RestoreContact(token));
                    app.bump_people();
                }
            },
            on_cancel: move |_| ctx.close_dialog(),
        }
    }
}

#[component]
fn DiscardEditConfirm(next: Option<Item>) -> Element {
    let mut ctx = use_context::<PeopleCtx>();
    rsx! {
        ConfirmDialog {
            title: "Discard changes?",
            body: "Your changes to this contact won't be saved.",
            confirm_label: "Discard",
            cancel_label: "Keep editing",
            on_confirm: move |_| {
                ctx.close_dialog();
                ctx.edit_dirty.set(false);
                ctx.editing.set(None);
                match next.clone() {
                    Some(item) => ctx.select_only(item, true),
                    None => ctx.clear_selection(),
                }
            },
            on_cancel: move |_| ctx.close_dialog(),
        }
    }
}

#[component]
fn NewCategoryDialog(ids: Vec<ContactId>) -> Element {
    let app = use_app();
    let ctx = use_context::<PeopleCtx>();
    let mut name = use_signal(String::new);
    let existing = store().categories().unwrap_or_default();
    let taken = existing.iter().any(|c| c.name.eq_ignore_ascii_case(name.read().trim()));
    let can_save = !name.read().trim().is_empty() && !taken;
    let save = move || {
        let n = name.peek().trim().to_string();
        if n.is_empty() {
            return;
        }
        let existing = store().categories().unwrap_or_default();
        // Next preset colour not in use yet.
        let color = CATEGORY_COLORS
            .iter()
            .map(|(c, _, _)| *c)
            .find(|c| !existing.iter().any(|e| e.color == *c))
            .unwrap_or("gray")
            .to_string();
        if app.check(store().upsert_category(&Category { name: n.clone(), color })).is_none() {
            return;
        }
        if !ids.is_empty() && app.check(store().toggle_contact_category(&ids, &n)).is_none() {
            return;
        }
        app.bump_people();
        ctx.close_dialog();
    };
    rsx! {
        Dialog {
            title: "New category",
            width: 400,
            on_close: move |_| ctx.close_dialog(),
            actions: rsx! {
                button { r#type: "button", class: "ax-btn ax-primary", disabled: !can_save, onclick: {
                    let save = save.clone();
                    move |_| save()
                }, "Save" }
                button { r#type: "button", class: "ax-btn", onclick: move |_| ctx.close_dialog(), "Cancel" }
            },
            label { class: "ax-people-input",
                span { class: "ax-field-label", "Name" }
                input {
                    class: "ax-input",
                    value: "{name}",
                    "aria-label": "Category name",
                    onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                    oninput: move |e| name.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        keys::stop_typing_keys(&e);
                        if e.key() == Key::Enter && can_save {
                            save();
                        }
                    },
                }
                if taken { span { class: "ax-people-error", role: "alert", "A category with this name already exists." } }
            }
        }
    }
}

/// Parsed import file waiting for "Import".
#[derive(Clone, Debug, PartialEq)]
struct ParsedImport {
    file: String,
    contacts: Vec<Contact>,
    lists: Vec<ContactList>,
    lossy: bool,
}

fn parse_import(file: &str, text: &str, lossy: bool) -> ParsedImport {
    let lower = file.to_lowercase();
    let is_csv = lower.ends_with(".csv") || (!lower.ends_with(".vcf") && !lower.ends_with(".vcard") && !text.contains("BEGIN:VCARD"));
    let (contacts, lists) = if is_csv { (Store::parse_contacts_csv(text), Vec::new()) } else { Store::parse_vcard(text) };
    ParsedImport { file: file.to_string(), contacts, lists, lossy }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "Imported 3 contacts, 1 merged, 2 skipped" (2.13).
fn import_toast((imported, merged, skipped): (usize, usize, usize)) -> String {
    let mut text = format!("Imported {}", plural(imported, "contact", "contacts"));
    if merged > 0 {
        text.push_str(&format!(", {merged} merged"));
    }
    if skipped > 0 {
        text.push_str(&format!(", {skipped} skipped"));
    }
    text
}

fn import_summary(p: &ParsedImport) -> String {
    let mut s = format!("Found {}", plural(p.contacts.len(), "contact", "contacts"));
    if !p.lists.is_empty() {
        s.push_str(&format!(" and {}", plural(p.lists.len(), "contact list", "contact lists")));
    }
    s.push('.');
    s
}

#[component]
fn ImportDialog() -> Element {
    let app = use_app();
    let ctx = use_context::<PeopleCtx>();
    let mut parsed = use_signal(|| None::<ParsedImport>);
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut folder = use_signal(|| match &*ctx.node.peek() {
        Node::Folder(f) => f.clone(),
        _ => String::new(),
    });
    let mut merge = use_signal(|| false);
    let folders = store().contact_folders().unwrap_or_default();
    let browse = move |_| {
        busy.set(true);
        spawn(async move {
            let picked = io::pick_file(".vcf,.vcard,.csv,text/vcard,text/csv").await;
            busy.set(false);
            let Some(file) = picked else { return };
            match Store::decode_import_file(&file.data) {
                Ok((text, lossy)) => {
                    let p = parse_import(&file.name, &text, lossy);
                    if p.contacts.is_empty() && p.lists.is_empty() {
                        error.set(Some(format!("No contacts were found in {}.", file.name)));
                        parsed.set(None);
                    } else {
                        error.set(None);
                        parsed.set(Some(p));
                    }
                }
                Err(e) => error.set(Some(e.to_string())),
            }
        });
    };
    let import = move |_| {
        let Some(p) = parsed() else { return };
        if let Some(counts) = app.check(store().import_contacts_merging(p.contacts, p.lists, &folder(), merge())) {
            app.toast(import_toast(counts));
            app.bump_people();
            ctx.close_dialog();
        }
    };
    let p = parsed();
    rsx! {
        Dialog {
            title: "Import contacts",
            width: 560,
            on_close: move |_| ctx.close_dialog(),
            actions: rsx! {
                button {
                    r#type: "button",
                    class: "ax-btn ax-primary",
                    disabled: p.is_none(),
                    title: if p.is_none() { "Choose a file first" } else { "" },
                    onclick: import,
                    "Import"
                }
                button { r#type: "button", class: "ax-btn", onclick: move |_| ctx.close_dialog(), "Cancel" }
            },
            div { class: "ax-people-io",
                p { class: "ax-people-io-text", "Choose a vCard (.vcf) or CSV (.csv) file. You can check the contacts before they're added." }
                div { class: "ax-people-io-row",
                    button { r#type: "button", class: "ax-btn", disabled: busy(), onclick: browse,
                        Icon { name: "folder_open" }
                        span { "Browse" }
                    }
                    if let Some(p) = &p { span { class: "ax-people-io-file ax-ellipsis", "{p.file}" } }
                }
                if let Some(err) = error() {
                    div { class: "ax-message-bar ax-error", role: "alert", Icon { name: "error_circle" } span { "{err}" } }
                }
                if let Some(p) = p {
                    if p.lossy {
                        div { class: "ax-message-bar ax-warning", role: "status",
                            Icon { name: "warning" }
                            span { "Some characters may not display correctly. If names look wrong, cancel and save the file as UTF-8." }
                        }
                    }
                    div { class: "ax-people-io-summary", "{import_summary(&p)}" }
                    table { class: "ax-people-preview",
                        thead { tr { th { "Name" } th { "Email" } th { "Company" } } }
                        tbody {
                            for (i, c) in p.contacts.iter().take(10).enumerate() {
                                tr { key: "{i}",
                                    td { class: "ax-ellipsis", "{super::view::display_name(c, false)}" }
                                    td { class: "ax-ellipsis", "{c.primary_email().unwrap_or_default()}" }
                                    td { class: "ax-ellipsis", "{c.company}" }
                                }
                            }
                        }
                    }
                    if p.contacts.len() > 10 { div { class: "ax-people-muted", "Showing the first 10." } }
                    label { class: "ax-people-input",
                        span { class: "ax-field-label", "Destination folder" }
                        select {
                            class: "ax-select",
                            "aria-label": "Destination folder",
                            onchange: move |e| folder.set(e.value()),
                            option { value: "", selected: folder().is_empty(), "Contacts" }
                            for f in folders.iter().cloned() {
                                option { key: "{f}", value: "{f}", selected: folder() == f, "{f}" }
                            }
                        }
                    }
                    label { class: "ax-people-input",
                        span { class: "ax-field-label", "Duplicates" }
                        select {
                            class: "ax-select",
                            "aria-label": "Duplicates",
                            onchange: move |e| merge.set(e.value() == "merge"),
                            option { value: "skip", selected: !merge(), "Skip contacts already saved with the same name and email" }
                            option { value: "merge", selected: merge(), "Merge into saved contacts with the same email address" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ExportDialog() -> Element {
    let app = use_app();
    let ctx = use_context::<PeopleCtx>();
    let mut source = use_signal(|| "*".to_string());
    let mut format = use_signal(|| "vcf4".to_string());
    let folders = store().contact_folders().unwrap_or_default();
    let export = move |_| {
        let src = source();
        let contacts: Vec<Contact> = store().contacts().unwrap_or_default().into_iter().filter(|c| src == "*" || c.folder == src).collect();
        // Lists live in the default folder (2.9).
        let lists = if src == "*" || src.is_empty() { store().contact_lists().unwrap_or_default() } else { Vec::new() };
        let fmt = format();
        // CSV has no way to express contact lists.
        let lists = if fmt == "csv" { Vec::new() } else { lists };
        let (text, ext) = match fmt.as_str() {
            "csv" => (Store::contacts_to_csv(&contacts), "csv"),
            v => (Store::contacts_to_vcard(&contacts, &lists, v == "vcf4"), "vcf"),
        };
        if contacts.is_empty() && lists.is_empty() {
            app.error("There are no contacts in this folder.");
            return;
        }
        match io::save_download("contacts", ext, &text) {
            Ok(path) => {
                let mut what = plural(contacts.len(), "contact", "contacts");
                if !lists.is_empty() {
                    what.push_str(&format!(" and {}", plural(lists.len(), "contact list", "contact lists")));
                }
                let folder = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(io::download_dir);
                app.toast_with_action(
                    format!("Exported {what} to {}", io::display_path(&path)),
                    "Open folder",
                    crate::state::ToastAction::OpenPath(folder),
                );
                ctx.close_dialog();
            }
            Err(e) => {
                app.error(format!("Couldn't export: {e}"));
            }
        }
    };
    let downloads = io::display_path(&io::download_dir());
    rsx! {
        Dialog {
            title: "Export contacts",
            width: 480,
            on_close: move |_| ctx.close_dialog(),
            actions: rsx! {
                button { r#type: "button", class: "ax-btn ax-primary", onclick: export, "Export" }
                button { r#type: "button", class: "ax-btn", onclick: move |_| ctx.close_dialog(), "Cancel" }
            },
            div { class: "ax-people-io",
                label { class: "ax-people-input",
                    span { class: "ax-field-label", "Contacts from this folder" }
                    select {
                        class: "ax-select",
                        "aria-label": "Contacts from this folder",
                        onchange: move |e| source.set(e.value()),
                        option { value: "*", selected: source() == "*", "All contacts" }
                        option { value: "", selected: source().is_empty(), "Contacts" }
                        for f in folders.iter().cloned() {
                            option { key: "{f}", value: "{f}", selected: source() == f, "{f}" }
                        }
                    }
                }
                label { class: "ax-people-input",
                    span { class: "ax-field-label", "Format" }
                    select {
                        class: "ax-select",
                        "aria-label": "Format",
                        onchange: move |e| format.set(e.value()),
                        option { value: "vcf4", selected: format() == "vcf4", "vCard 4.0 (.vcf)" }
                        option { value: "vcf3", selected: format() == "vcf3", "vCard 3.0 (.vcf)" }
                        option { value: "csv", selected: format() == "csv", "CSV (.csv)" }
                    }
                }
                p { class: "ax-people-io-text", "The file is saved to your Downloads folder ({downloads})." }
            }
        }
    }
}

/// People shortcuts (keyboard spec 7), shown by Shift+? and Help.
const SHORTCUTS: &[(&str, &[&[&str]])] = &[
    ("Create contact", &[&["Ctrl", "N"]]),
    ("Create contact list", &[&["Ctrl", "L"]]),
    ("Edit contact or list", &[&["Shift", "E"]]),
    ("Save contact or list", &[&["Ctrl", "S"]]),
    ("Discard edits / unselect all", &[&["Esc"]]),
    ("Delete contact or list", &[&["Delete"], &["Ctrl", "D"]]),
    ("Search contacts", &[&["Ctrl", "E"]]),
    ("Select all", &[&["Ctrl", "A"]]),
    ("Move in list", &[&["Up"], &["Down"]]),
    ("Add contacts to selection", &[&["Shift", "Up"], &["Shift", "Down"]]),
    ("Toggle checkbox", &[&["Space"]]),
    ("Context menu", &[&["Shift", "F10"]]),
    ("Next / previous region", &[&["F6"], &["Shift", "F6"]]),
    ("Show People shortcuts", &[&["Shift", "?"]]),
];

#[component]
fn ShortcutsDialog() -> Element {
    let mut app = use_app();
    let ctx = use_context::<PeopleCtx>();
    let mut filter = use_signal(String::new);
    let q = filter().to_lowercase();
    rsx! {
        Dialog {
            title: "Keyboard shortcuts",
            width: 560,
            on_close: move |_| ctx.close_dialog(),
            div { class: "ax-people-shortcuts",
                div { class: "ax-people-io-text",
                    "Shortcut set: Outlook "
                    button {
                        r#type: "button",
                        class: "ax-people-link",
                        onclick: move |_| {
                            ctx.close_dialog();
                            app.settings_dialog.set(Some(SettingsPage::General));
                        },
                        "Change"
                    }
                }
                input {
                    class: "ax-input",
                    placeholder: "Search shortcuts",
                    "aria-label": "Search shortcuts",
                    value: "{filter}",
                    oninput: move |e| filter.set(e.value()),
                    onkeydown: move |e| keys::stop_typing_keys(&e),
                }
                h3 { class: "ax-people-shortcuts-section", "People" }
                for (label, combos) in SHORTCUTS.iter().filter(|(l, _)| q.is_empty() || l.to_lowercase().contains(&q)) {
                    div { key: "{label}", class: "ax-people-shortcut",
                        span { class: "ax-grow", "{label}" }
                        span { class: "ax-people-keys",
                            for (i, combo) in combos.iter().enumerate() {
                                if i > 0 { span { class: "ax-people-keys-or", " or " } }
                                for (j, k) in combo.iter().enumerate() {
                                    if j > 0 { span { class: "ax-people-keys-plus", "+" } }
                                    kbd { class: "ax-people-key", "{k}" }
                                }
                            }
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
    fn delete_wording_follows_the_spec() {
        let c = |n: &str| Contact { display_name: n.into(), ..Default::default() };
        let l = ContactList::default();
        assert_eq!(delete_texts(&[c("Ada Lovelace")], &[]), ("Delete contact?".into(), "Ada Lovelace will be deleted from your contacts.".into()));
        assert_eq!(delete_texts(&[c("a"), c("b")], &[]).0, "Delete 2 contacts?");
        assert_eq!(delete_texts(&[], std::slice::from_ref(&l)), ("Delete contact list?".into(), "The people in this list won't be deleted, only the list.".into()));
    }

    #[test]
    fn import_toast_wording() {
        assert_eq!(import_toast((1, 0, 0)), "Imported 1 contact");
        assert_eq!(import_toast((3, 1, 2)), "Imported 3 contacts, 1 merged, 2 skipped");
        assert_eq!(import_toast((0, 0, 4)), "Imported 0 contacts, 4 skipped");
    }

    #[test]
    fn import_format_detection() {
        let v = parse_import("people.vcf", "BEGIN:VCARD\nVERSION:3.0\nFN:Ada\nEND:VCARD\n", false);
        assert_eq!(v.contacts.len(), 1);
        let c = parse_import("people.csv", "First Name,Last Name\nAda,Lovelace\n", false);
        assert_eq!(c.contacts[0].display_name, "Ada Lovelace");
        let sniffed = parse_import("export", "BEGIN:VCARD\nVERSION:4.0\nFN:Grace\nEND:VCARD\n", false);
        assert_eq!(sniffed.contacts[0].display_name, "Grace");
    }
}
