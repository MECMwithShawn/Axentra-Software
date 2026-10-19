//! Settings > Accounts pages other than Email accounts (which the sync
//! area owns): Signatures, Categories, Automatic replies, Shared with me,
//! Storage.

use axentra_core::*;
use dioxus::prelude::*;

use super::controls::{Note, Row, Section, Unavailable, use_settings};
use crate::kit::{Button, Checkbox, ConfirmDialog, IconButton, Menu, MenuItem, Switch, TextField, Variant, category_colors, dom, keys};
use crate::state::{store, use_app};

// ---------------------------------------------------------------- Signatures

const FORMAT_COMMANDS: &[(&str, &str, &str)] = &[
    ("bold", "text_bold", "Bold (Ctrl+B)"),
    ("italic", "text_italic", "Italic (Ctrl+I)"),
    ("underline", "text_underline", "Underline (Ctrl+U)"),
    ("insertUnorderedList", "text_bullet_list_ltr", "Bullets"),
    ("removeFormat", "text_clear_formatting", "Clear formatting"),
];

/// Signature HTML made safe to place in the app's own page: sanitised like
/// mail, and without `<style>` blocks (they would restyle the whole app).
pub fn safe_inline_html(raw: &str) -> String {
    let mut html = axentra_core::html::sanitize_email_html(raw, true).html;
    while let Some(start) = html.to_ascii_lowercase().find("<style") {
        let end = html.to_ascii_lowercase()[start..].find("</style>").map(|e| start + e + "</style>".len()).unwrap_or(html.len());
        html.replace_range(start..end, "");
    }
    html
}

#[component]
pub fn SignaturesPage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    let id = dom::use_unique_id("ax-signature");
    // The editor owns its DOM while typing; the draft follows it. When the
    // draft is reset from outside (Discard), the editor content is replaced.
    let mut shown = use_signal(|| s.signature_html.clone());
    let initial = use_hook(|| safe_inline_html(&s.signature_html));
    {
        let id = id.clone();
        use_effect(move || {
            let html = ctx.draft.read().signature_html.clone();
            if html != *shown.peek() {
                shown.set(html.clone());
                let safe = safe_inline_html(&html);
                dom::run(&format!("var e=document.getElementById({});if(e)e.innerHTML={};", dom::js_str(&id), dom::js_str(&safe)));
            }
        });
    }
    let read_back = {
        let id = id.clone();
        move || {
            let id = id.clone();
            spawn(async move {
                let js = format!("var e=document.getElementById({});return e?e.innerHTML:'';", dom::js_str(&id));
                if let Ok(html) = document::eval(&js).join::<String>().await {
                    let html = if html == "<br>" { String::new() } else { html };
                    shown.set(html.clone());
                    ctx.edit(|s| s.signature_html = html);
                }
            });
        }
    };
    rsx! {
        Section { title: "Email signature", description: "Added to the end of your messages.",
            div { class: "ax-sig-toolbar", role: "toolbar", "aria-label": "Formatting", "data-ax-roving": "horizontal",
                for (cmd, icon, label) in FORMAT_COMMANDS.iter().copied() {
                    button {
                        key: "{cmd}",
                        r#type: "button",
                        class: "ax-icon-btn ax-small",
                        title: "{label}",
                        "aria-label": "{label}",
                        "data-ax-item": "true",
                        "data-ax-keep-focus": "true",
                        tabindex: if cmd == "bold" { "0" } else { "-1" },
                        onclick: {
                            let read_back = read_back.clone();
                            move |_| {
                                dom::run(&format!("document.execCommand({})", dom::js_str(cmd)));
                                read_back();
                            }
                        },
                        crate::kit::Icon { name: icon, size: 16 }
                    }
                }
            }
            div {
                id: "{id}",
                class: "ax-sig-editor ax-selectable",
                contenteditable: "true",
                role: "textbox",
                "aria-multiline": "true",
                "aria-label": "Email signature",
                "data-placeholder": "Type your signature",
                dangerous_inner_html: "{initial}",
                oninput: {
                    let read_back = read_back.clone();
                    move |_| read_back()
                },
                onkeydown: move |e| keys::stop_typing_keys(&e),
            }
        }
        Section { title: "Select default signatures",
            Checkbox {
                label: "Include my signature on new messages I compose",
                checked: s.signature_on_new,
                onchange: move |v| ctx.edit(|s| s.signature_on_new = v),
            }
            Checkbox {
                label: "Include my signature on messages I forward or reply to",
                checked: s.signature_on_reply,
                onchange: move |v| ctx.edit(|s| s.signature_on_reply = v),
            }
        }
    }
}

// ---------------------------------------------------------------- Categories

#[component]
pub fn CategoriesPage() -> Element {
    let app = use_app();
    let _ = app.mail_rev();
    let categories = store().categories().unwrap_or_default();
    let mut adding = use_signal(|| false);
    let mut name = use_signal(String::new);
    let mut color = use_signal(|| "blue".to_string());
    let mut deleting = use_signal(|| None::<String>);
    let exists = categories.iter().any(|c| c.name.eq_ignore_ascii_case(name().trim()));
    let mut create = move || {
        let n = name().trim().to_string();
        if n.is_empty() || exists {
            return;
        }
        if app.check(store().upsert_category(&Category { name: n.clone(), color: color() })).is_some() {
            app.bump_mail();
            app.bump_calendar();
            name.set(String::new());
            adding.set(false);
            dom::announce(&format!("Category {n} created"), false);
        }
    };
    rsx! {
        Section { title: "Categories", description: "Colour-code messages and events. Changes apply right away.",
            if adding() {
                div { class: "ax-cat-form",
                    TextField {
                        label: "Name",
                        value: name(),
                        autofocus: true,
                        hint: if exists { "A category with this name already exists." } else { "" },
                        oninput: move |v| name.set(v),
                        onkeydown: move |e: KeyboardEvent| match e.key() {
                            Key::Enter => create(),
                            Key::Escape => {
                                e.stop_propagation();
                                adding.set(false);
                            }
                            _ => {}
                        },
                    }
                    div { class: "ax-cat-colors", role: "radiogroup", "aria-label": "Colour", "data-ax-roving": "grid", "data-ax-cols": "13",
                        for (c, light, _) in CATEGORY_COLORS.iter().copied() {
                            button {
                                key: "{c}",
                                r#type: "button",
                                role: "radio",
                                class: "ax-cat-color",
                                class: if color() == c { "ax-selected" },
                                "aria-checked": if color() == c { "true" } else { "false" },
                                "aria-label": "{c}",
                                title: "{c}",
                                "data-ax-item": "true",
                                tabindex: if color() == c { "0" } else { "-1" },
                                style: "background:{light}",
                                onclick: move |_| color.set(c.to_string()),
                            }
                        }
                    }
                    div { class: "ax-row", style: "gap:8px",
                        Button {
                            label: "Save",
                            variant: Variant::Primary,
                            disabled: name().trim().is_empty() || exists,
                            title: if name().trim().is_empty() { "Type a name first" } else if exists { "Choose a different name" } else { "Create the category" },
                            onclick: move |_| create(),
                        }
                        Button { label: "Cancel", onclick: move |_| adding.set(false) }
                    }
                }
            } else {
                Button { label: "Create category", icon: "add", onclick: move |_| adding.set(true) }
            }
            if categories.is_empty() {
                Note { text: "You don't have any categories yet." }
            }
            div { class: "ax-cat-list", role: "list",
                for c in categories.iter().cloned() {
                    CategoryRow { key: "{c.name}", category: c, on_delete: move |n| deleting.set(Some(n)) }
                }
            }
        }
        if let Some(n) = deleting() {
            ConfirmDialog {
                title: "Delete category?",
                message: format!("\"{n}\" will be removed from the category list. Items that use it keep the name."),
                confirm_label: "Delete",
                danger: true,
                on_cancel: move |_| deleting.set(None),
                on_confirm: move |_| {
                    let n = n.clone();
                    deleting.set(None);
                    if app.check(store().delete_category(&n)).is_some() {
                        app.bump_mail();
                        app.bump_calendar();
                        app.toast(format!("Deleted category \"{n}\""));
                    }
                },
            }
        }
    }
}

#[component]
fn CategoryRow(category: Category, on_delete: EventHandler<String>) -> Element {
    let app = use_app();
    let mut menu_at = use_signal(|| None::<(f64, f64)>);
    let (light, _) = category_colors(&category.color);
    let items: Vec<MenuItem> = CATEGORY_COLORS
        .iter()
        .map(|(c, l, _)| MenuItem::new(*c, c.to_string()).swatch(*l).radio(*c == category.color))
        .collect();
    let name = category.name.clone();
    rsx! {
        div { class: "ax-cat-row", role: "listitem",
            button {
                r#type: "button",
                class: "ax-cat-swatch",
                style: "background:{light}",
                title: "Change colour",
                "aria-label": "Change the colour of {category.name}",
                "aria-haspopup": "menu",
                onclick: move |e: MouseEvent| {
                    let c = e.client_coordinates();
                    menu_at.set(Some((c.x, c.y + 12.0)));
                },
            }
            span { class: "ax-grow ax-ellipsis", "{category.name}" }
            IconButton { icon: "delete", title: "Delete {category.name}", small: true, onclick: move |_| on_delete.call(name.clone()) }
        }
        if let Some((x, y)) = menu_at() {
            Menu {
                items,
                x,
                y,
                label: "Colour",
                on_close: move |_| menu_at.set(None),
                on_select: {
                    let name = category.name.clone();
                    move |c: String| {
                        if app.check(store().upsert_category(&Category { name: name.clone(), color: c })).is_some() {
                            app.bump_mail();
                            app.bump_calendar();
                        }
                    }
                },
            }
        }
    }
}

// ---------------------------------------------------------------- Others

#[component]
pub fn AutomaticRepliesPage() -> Element {
    let app = use_app();
    let _ = app.accounts_rev();
    let microsoft = store().accounts().unwrap_or_default().iter().any(|a| a.kind == AccountKind::Microsoft);
    rsx! {
        Section { title: "Automatic replies", description: "Let people know you're away by replying automatically.",
            Switch {
                label: "Turn on automatic replies",
                checked: false,
                disabled: true,
                title: if microsoft {
                    "Automatic replies for Microsoft accounts are managed by the sync engine, which doesn't support them yet"
                } else {
                    "Automatic replies are set on the mail server. They're available for Microsoft 365 and Outlook.com accounts."
                },
                onchange: move |_| {},
            }
            Note { text: "IMAP accounts: set up an out-of-office reply with your email provider." }
        }
    }
}

#[component]
pub fn SharedWithMePage() -> Element {
    rsx! {
        Unavailable {
            icon: "people",
            title: "Nothing has been shared with you",
            text: "Mailboxes and calendars that others share with you appear here.",
        }
    }
}

/// Totals for the Storage page.
#[derive(Debug, Default, PartialEq)]
pub struct Usage {
    pub messages: i64,
    pub deleted: i64,
    pub contacts: usize,
    pub open_tasks: usize,
}

pub fn usage(store: &axentra_core::Store) -> Usage {
    let folders = store.folders(None).unwrap_or_default();
    Usage {
        messages: folders.iter().map(|f| f.total_count).sum(),
        deleted: folders.iter().filter(|f| f.role == FolderRole::Deleted).map(|f| f.total_count).sum(),
        contacts: store.contacts().map(|c| c.len()).unwrap_or(0),
        open_tasks: store.task_list_counts().map(|m| m.values().sum()).unwrap_or(0),
    }
}

#[component]
pub fn StoragePage() -> Element {
    let app = use_app();
    let _ = (app.mail_rev(), app.people_rev(), app.tasks_rev());
    let u = usage(store());
    let size = store().path().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len() as i64);
    let mut confirm = use_signal(|| false);
    rsx! {
        Section { title: "Storage", description: "What Axentra Outlook keeps on this device.",
            Row { label: "Storage used on this device",
                span { class: "ax-set-value", match size { Some(b) => crate::fmt::file_size(b), None => "In memory (preview)".into() } }
            }
            Row { label: "Messages", span { class: "ax-set-value", "{u.messages}" } }
            Row { label: "Contacts", span { class: "ax-set-value", "{u.contacts}" } }
            Row { label: "Open tasks", span { class: "ax-set-value", "{u.open_tasks}" } }
            Row { label: "Deleted Items", description: "Messages in Deleted Items still use space.",
                Button {
                    label: "Empty Deleted Items",
                    icon: "delete",
                    disabled: u.deleted == 0,
                    title: if u.deleted == 0 { "Deleted Items is already empty".to_string() } else { format!("Permanently delete {} items", u.deleted) },
                    onclick: move |_| confirm.set(true),
                }
            }
        }
        if confirm() {
            ConfirmDialog {
                title: "Empty Deleted Items?",
                message: format!("{} items will be permanently deleted. This can't be undone.", u.deleted),
                confirm_label: "Delete all",
                danger: true,
                on_cancel: move |_| confirm.set(false),
                on_confirm: move |_| {
                    confirm.set(false);
                    let s = store();
                    let mut ok = true;
                    for f in s.folders(None).unwrap_or_default().iter().filter(|f| f.role == FolderRole::Deleted) {
                        ok &= app.check(s.empty_folder(f.id)).is_some();
                    }
                    app.bump_mail();
                    if ok {
                        app.toast("Deleted Items emptied");
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_html_cannot_restyle_or_script_the_app() {
        let raw = r#"<p onclick="x()">Hi<script>alert(1)</script></p><style>body{display:none}</style><b>Bye</b><STYLE>.ax-app{}</STYLE>"#;
        let safe = safe_inline_html(raw);
        assert!(!safe.to_lowercase().contains("<style") && !safe.contains("script") && !safe.contains("onclick"), "{safe}");
        assert!(safe.contains("Hi") && safe.contains("<b>Bye</b>"));
    }

    #[test]
    fn usage_counts_demo_data() {
        let store = axentra_core::Store::open_in_memory().unwrap();
        axentra_core::seed::seed_demo(&store, chrono::Local::now()).unwrap();
        let u = usage(&store);
        assert!(u.messages >= 20);
        assert!(u.contacts > 0);
        assert!(u.open_tasks > 0);
    }
}
