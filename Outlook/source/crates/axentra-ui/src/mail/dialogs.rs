//! Mail dialogs: confirmations, folder picker, sweep, block, report,
//! ignore, message details, categories, snooze / flag dates and the
//! keyboard shortcuts sheet. Copy is Axentra's own where the spec says so.

use axentra_core::*;
use chrono::{Local, NaiveDate, NaiveTime, TimeZone, Utc};
use dioxus::prelude::*;

use super::actions::{self, items};
use super::ctx::{MailCtx, MailDialog, use_mail, use_model};
use crate::kit::{Button, Checkbox, Dialog, Icon, Select, TextField, Variant, keys};
use crate::state::{AppState, store, use_app};

#[component]
pub fn MailDialogs() -> Element {
    let ctx = use_mail();
    let Some(dialog) = ctx.dialog.read().clone() else { return rsx! {} };
    match dialog {
        MailDialog::DeleteFolder(id) => rsx! { DeleteFolderDialog { id } },
        MailDialog::EmptyFolder(id) => rsx! { EmptyFolderDialog { id, permanent: true } },
        MailDialog::DeleteAll(id) => rsx! { EmptyFolderDialog { id, permanent: false } },
        MailDialog::PermanentDelete(ids) => rsx! { PermanentDeleteDialog { ids } },
        MailDialog::MoveTo { ids, copy } => rsx! { FolderPicker { ids, copy, moving_folder: None } },
        MailDialog::MoveFolder(id) => rsx! { FolderPicker { ids: Vec::new(), copy: false, moving_folder: Some(id) } },
        MailDialog::Sweep(id) => rsx! { SweepDialog { id } },
        MailDialog::Block(id) => rsx! { BlockDialog { id } },
        MailDialog::Report { ids, phishing } => rsx! { ReportDialog { ids, phishing } },
        MailDialog::Ignore(conv) => rsx! { IgnoreDialog { conversation: conv } },
        MailDialog::Details(id) => rsx! { DetailsDialog { id } },
        MailDialog::NewCategory(ids) => rsx! { NewCategoryDialog { ids } },
        MailDialog::ManageCategories => rsx! { ManageCategoriesDialog {} },
        MailDialog::SnoozeUntil(ids) => rsx! { DateDialog { ids, snooze: true } },
        MailDialog::FlagCustom(ids) => rsx! { DateDialog { ids, snooze: false } },
        MailDialog::SetUpArchive(ids) => rsx! { SetUpArchiveDialog { ids } },
        MailDialog::Shortcuts => rsx! { ShortcutsDialog {} },
        MailDialog::Help => rsx! { HelpDialog {} },
    }
}

fn close(mut ctx: MailCtx) {
    ctx.dialog.set(None);
}

/// Keys typed in a dialog field are not mail shortcuts.
fn typing(ctx: MailCtx) -> impl Fn(KeyboardEvent) {
    move |e| ctx.mark_typing_for(&e)
}

#[component]
fn Confirm(title: String, body: String, confirm: String, #[props(default)] danger: bool, on_confirm: EventHandler<()>, children: Element) -> Element {
    let ctx = use_mail();
    rsx! {
        Dialog {
            title,
            width: 448,
            on_close: move |_| close(ctx),
            actions: rsx! {
                Button { label: confirm, variant: Variant::Primary, onclick: move |_| on_confirm.call(()) }
                Button { label: "Cancel", onclick: move |_| close(ctx) }
            },
            p { class: "ax-mail-dialog-text", "{body}" }
            {children}
        }
    }
}

#[component]
fn DeleteFolderDialog(id: FolderId) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let s = store();
    let Some(f) = s.folder(id).ok().flatten() else { return rsx! {} };
    let deleted = s.folder_by_role(f.account_id, FolderRole::Deleted).ok().flatten().map(|d| d.id);
    let inside = deleted.is_some_and(|d| s.folder_is_within(id, d).unwrap_or(false));
    let body = if inside {
        format!("\"{}\" and everything in it will be permanently deleted.", f.name)
    } else {
        format!("\"{}\" and everything in it will be moved to Deleted Items.", f.name)
    };
    let name = f.name.clone();
    rsx! {
        Confirm {
            title: "Delete folder?",
            body,
            confirm: "Delete",
            on_confirm: move |_| {
                let mut ctx = ctx;
                close(ctx);
                if app.check(store().delete_folder(id)).is_some() {
                    if *ctx.folder.peek() == Some(id) {
                        ctx.folder.set(None);
                    }
                    app.toast(format!("Deleted folder {name}"));
                    app.bump_mail();
                }
            },
        }
    }
}

#[component]
fn EmptyFolderDialog(id: FolderId, permanent: bool) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let Some(f) = store().folder(id).ok().flatten() else { return rsx! {} };
    let (title, body, confirm) = if permanent {
        ("Empty folder?".to_string(), format!("All {} in {} will be permanently deleted. You can't undo this.", items(f.total_count as usize), f.name), "Delete")
    } else {
        ("Delete all?".to_string(), format!("All {} in {} will be moved to Deleted Items.", items(f.total_count as usize), f.name), "Delete all")
    };
    rsx! {
        Confirm {
            title,
            body,
            confirm,
            on_confirm: move |_| {
                let ctx = ctx;
                close(ctx);
                let s = store();
                if permanent {
                    if app.check(s.empty_folder(id)).is_some() {
                        ctx.clear_selection();
                        app.toast("Folder emptied");
                        app.bump_mail();
                    }
                } else if let Some(records) = app.check(s.delete_all_in_folder(id)) {
                    ctx.clear_selection();
                    let n = records.len();
                    if n > 0 {
                        let undo = crate::state::UndoAction::Moves(records);
                        ctx.push_undo(undo.clone());
                        app.toast_with_undo(format!("Deleted {}", items(n)), undo);
                    }
                    app.bump_mail();
                }
            },
        }
    }
}

#[component]
fn PermanentDeleteDialog(ids: Vec<MessageId>) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let model = use_model();
    let n = ids.len();
    let title = if n == 1 { "Permanently delete this item?".to_string() } else { format!("Permanently delete {n} items?") };
    let local = store()
        .messages_by_ids(&ids)
        .unwrap_or_default()
        .first()
        .and_then(|m| store().account(m.account_id).ok().flatten())
        .is_none_or(|a| a.kind != AccountKind::Microsoft);
    let body = if local {
        "This removes it from your mailbox and can't be undone.".to_string()
    } else {
        "It won't go to Deleted Items. You may be able to get it back from Recover deleted items for a limited time.".to_string()
    };
    rsx! {
        Confirm {
            title,
            body,
            confirm: "Delete",
            on_confirm: move |_| {
                close(ctx);
                let m = model.read();
                actions::delete_permanently(app, ctx, &m, ids.clone());
            },
        }
    }
}

/// Folder rows for pickers: (folder, depth), in tree order.
fn folder_rows(account: Option<AccountId>) -> Vec<(Folder, usize)> {
    let s = store();
    let folders = s.folders(account).unwrap_or_default();
    let snoozed = s.snoozed_folder_ids().unwrap_or_default();
    let accounts = s.accounts().unwrap_or_default();
    let prefs = super::ctx::FolderPrefs { favorites_hidden: true, ..Default::default() };
    let nodes = super::folders::build_nodes(&accounts, &folders, &[], &[], &snoozed, &prefs, None);
    nodes
        .into_iter()
        .filter_map(|n| match n.kind {
            super::folders::NodeKind::Folder(f) => Some((f, n.level.saturating_sub(2) as usize)),
            _ => None,
        })
        .collect()
}

/// "Move to a different folder" / "Copy to a different folder" / "Move folder".
#[component]
fn FolderPicker(ids: Vec<MessageId>, copy: bool, moving_folder: Option<FolderId>) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let model = use_model();
    let mut query = use_signal(String::new);
    let mut chosen = use_signal(|| None::<FolderId>);
    let mut new_name = use_signal(|| None::<String>);
    let mut error = use_signal(|| None::<String>);
    let s = store();
    let account = match moving_folder {
        Some(f) => s.folder(f).ok().flatten().map(|f| f.account_id),
        None => s.messages_by_ids(&ids).unwrap_or_default().first().map(|m| m.account_id),
    };
    let current = *ctx.folder.read();
    let q = query().to_lowercase();
    let rows: Vec<(Folder, usize, bool)> = folder_rows(account)
        .into_iter()
        .filter(|(f, _)| q.is_empty() || f.name.to_lowercase().contains(&q))
        .map(|(f, d)| {
            let disabled = match moving_folder {
                Some(m) => s.folder_is_within(f.id, m).unwrap_or(true),
                None => matches!(f.role, FolderRole::Notes | FolderRole::ConversationHistory | FolderRole::Outbox) || (!copy && Some(f.id) == current),
            };
            (f, d, disabled)
        })
        .collect();
    let title = match (moving_folder.is_some(), copy) {
        (true, _) => "Move folder",
        (false, true) => "Copy to a different folder",
        (false, false) => "Move to a different folder",
    };
    let verb = if copy { "Copy" } else { "Move" };
    let ids_sig = use_signal(|| ids.clone());
    let confirm = move |target: Option<FolderId>| {
        let ids2 = ids_sig.peek().clone();
        let mut ctx = ctx;
        match moving_folder {
            Some(f) => {
                ctx.dialog.set(None);
                if app.check(store().move_folder(f, target)).is_some() {
                    app.toast("Folder moved");
                    app.bump_mail();
                }
            }
            None => {
                let Some(t) = target else { return };
                ctx.dialog.set(None);
                let m = model.read();
                if copy {
                    actions::copy_to(app, ids2.clone(), t);
                } else {
                    actions::move_to(app, ctx, &m, ids2.clone(), t);
                }
            }
        }
    };
    let mut create = move |name: String| {
        let name = name.trim().to_string();
        if name.is_empty() {
            new_name.set(None);
            return;
        }
        let Some(a) = account else { return };
        let parent = *chosen.peek();
        if let Some(err) = super::folders::validate_name(&name, a, parent, None) {
            error.set(Some(err));
            return;
        }
        match store().create_folder(a, parent, &name) {
            Ok(id) => {
                new_name.set(None);
                error.set(None);
                chosen.set(Some(id));
                app.bump_mail();
            }
            Err(_) => {
                app.error("Couldn't create the folder. Try again.");
            }
        }
    };
    let can_confirm = moving_folder.is_some() || chosen().is_some();
    rsx! {
        Dialog {
            title,
            width: 440,
            on_close: move |_| close(ctx),
            actions: rsx! {
                if new_name().is_none() && moving_folder.is_none() {
                    Button { label: "New folder", icon: "folder_add", variant: Variant::Subtle, class: "ax-mail-dialog-left", onclick: move |_| new_name.set(Some(String::new())) }
                }
                Button {
                    label: if moving_folder.is_some() { "Move" } else { verb },
                    variant: Variant::Primary,
                    disabled: !can_confirm,
                    onclick: move |_| confirm(chosen()),
                }
                Button { label: "Cancel", onclick: move |_| close(ctx) }
            },
            TextField {
                value: query(),
                placeholder: "Search for a folder",
                autofocus: true,
                oninput: move |v| query.set(v),
                onkeydown: typing(ctx),
            }
            div { class: "ax-mail-picker", role: "listbox", "aria-label": "Folders",
                if moving_folder.is_some() && q.is_empty() {
                    button {
                        r#type: "button",
                        class: "ax-mail-picker-row",
                        class: if chosen().is_none() { "ax-selected" },
                        role: "option",
                        onclick: move |_| chosen.set(None),
                        ondoubleclick: move |_| confirm(None),
                        Icon { name: "mail", size: 20 }
                        span { "Top level of the mailbox" }
                    }
                }
                for (f, depth, disabled) in rows {
                    button {
                        key: "{f.id}",
                        r#type: "button",
                        class: "ax-mail-picker-row",
                        class: if chosen() == Some(f.id) { "ax-selected" },
                        role: "option",
                        "aria-selected": if chosen() == Some(f.id) { "true" } else { "false" },
                        disabled,
                        style: "padding-left:{12 + depth * 16}px",
                        onclick: move |_| chosen.set(Some(f.id)),
                        ondoubleclick: move |_| confirm(Some(f.id)),
                        Icon { name: f.role.icon(), size: 20 }
                        span { class: "ax-ellipsis", "{f.name}" }
                    }
                }
                if let Some(name) = new_name() {
                    div { class: "ax-mail-picker-new",
                        Icon { name: "folder_add", size: 20 }
                        input {
                            class: "ax-input",
                            value: "{name}",
                            placeholder: "New folder",
                            "aria-label": "New folder name",
                            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                            oninput: move |e| {
                                new_name.set(Some(e.value()));
                                error.set(None);
                            },
                            onkeydown: move |e: KeyboardEvent| {
                                ctx.mark_typing_for(&e);
                                keys::stop_typing_keys(&e);
                                match e.key() {
                                    Key::Enter => create(new_name.peek().clone().unwrap_or_default()),
                                    Key::Escape => {
                                        e.stop_propagation();
                                        new_name.set(None);
                                    }
                                    _ => {}
                                }
                            },
                        }
                    }
                    if let Some(err) = error() { div { class: "ax-mail-node-error", role: "alert", "{err}" } }
                }
            }
        }
    }
}

#[component]
fn SweepDialog(id: MessageId) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let model = use_model();
    let s = store();
    let Some(m) = s.message(id).ok().flatten() else { return rsx! {} };
    let folder = s.folder(m.folder_id).ok().flatten();
    let folder_name = folder.as_ref().map(|f| f.name.clone()).unwrap_or_default();
    let deleted = s.folder_by_role(m.account_id, FolderRole::Deleted).ok().flatten().map(|f| f.id).unwrap_or_default();
    let mut mode = use_signal(|| "move_all".to_string());
    let mut target = use_signal(|| deleted);
    let options = [
        ("move_all", format!("Move all messages from the {folder_name} folder")),
        ("move_all_future", format!("Move all messages from the {folder_name} folder and any future messages")),
        ("keep_latest", "Always keep the latest message and move the rest".to_string()),
        ("older_than_10_days", "Always move messages older than 10 days".to_string()),
    ];
    let targets: Vec<(String, String)> = folder_rows(Some(m.account_id))
        .into_iter()
        .filter(|(f, _)| Some(f.id) != folder.as_ref().map(|x| x.id) && !matches!(f.role, FolderRole::Notes | FolderRole::ConversationHistory | FolderRole::Outbox | FolderRole::Drafts))
        .map(|(f, d)| (f.id.to_string(), format!("{}{}", "  ".repeat(d), f.name)))
        .collect();
    let sender = m.from.clone();
    let sender_name = m.from.display().to_string();
    let source = m.folder_id;
    rsx! {
        Dialog {
            title: "Sweep messages",
            width: 520,
            on_close: move |_| close(ctx),
            actions: rsx! {
                Button {
                    label: "Sweep",
                    variant: Variant::Primary,
                    onclick: move |_| {
                        close(ctx);
                        let mode = mode.peek().clone();
                        let t = *target.peek();
                        match store().sweep(source, &sender.address, &mode, t) {
                            Ok(records) => {
                                let m = model.read();
                                let ids: Vec<MessageId> = records.iter().map(|r| r.message_id).collect();
                                actions::select_after_removal(app, ctx, &m, &ids);
                                let mut text = format!("Swept {}", items(records.len()));
                                if mode != "move_all" {
                                    text.push_str(". The rule runs once a day.");
                                }
                                if records.is_empty() {
                                    app.toast(text);
                                } else {
                                    let undo = crate::state::UndoAction::Moves(records);
                                    ctx.push_undo(undo.clone());
                                    app.toast_with_undo(text, undo);
                                }
                                app.bump_mail();
                            }
                            Err(e) => {
                                app.error(e.to_string());
                            }
                        }
                    },
                }
                Button { label: "Cancel", onclick: move |_| close(ctx) }
            },
            p { class: "ax-mail-dialog-text",
                "Choose what to do with email from "
                b { "{sender_name}" }
                " ({m.from.address})."
            }
            div { class: "ax-mail-radios", role: "radiogroup", "aria-label": "Sweep options",
                for (value, label) in options {
                    label { class: "ax-mail-radio", key: "{value}",
                        input {
                            r#type: "radio",
                            name: "sweep",
                            checked: mode() == value,
                            onchange: move |_| mode.set(value.to_string()),
                        }
                        span { "{label}" }
                    }
                }
            }
            Select {
                label: "Move to",
                value: target().to_string(),
                options: targets,
                onchange: move |v: String| if let Ok(id) = v.parse() { target.set(id) },
            }
            if mode() != "move_all" {
                p { class: "ax-mail-dialog-note", Icon { name: "info", size: 16 } "This creates a rule that runs once a day. Sweep rules made in Axentra don't sync with Outlook." }
            }
        }
    }
}

#[component]
fn BlockDialog(id: MessageId) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let Some(m) = store().message(id).ok().flatten() else { return rsx! {} };
    let sender = m.from.clone();
    rsx! {
        Confirm {
            title: format!("Block {}?", m.from.display()),
            body: format!("Messages from {} will go to Junk Email. Messages you already have stay where they are.", m.from.address),
            confirm: "Block",
            on_confirm: move |_| {
                close(ctx);
                if app.check(store().block_sender(&sender)).is_some() {
                    app.toast(format!("Blocked {}", sender.address));
                }
            },
        }
    }
}

#[component]
fn ReportDialog(ids: Vec<MessageId>, phishing: bool) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let model = use_model();
    let mut block = use_signal(|| false);
    let msgs = store().messages_by_ids(&ids).unwrap_or_default();
    let senders: Vec<String> = {
        let mut v: Vec<String> = msgs.iter().map(|m| m.from.address.clone()).collect();
        v.sort();
        v.dedup();
        v
    };
    let who = if senders.len() == 1 { senders[0].clone() } else { format!("{} senders", senders.len()) };
    let (title, body) = if phishing {
        ("Report as phishing?", format!("{} will be moved to Deleted Items. This report stays on this device.", if ids.len() == 1 { "The message".to_string() } else { format!("{} messages", ids.len()) }))
    } else {
        ("Report as junk?", format!("{} will be moved to Junk Email. This report stays on this device.", if ids.len() == 1 { "The message".to_string() } else { format!("{} messages", ids.len()) }))
    };
    rsx! {
        Confirm {
            title,
            body,
            confirm: "Report",
            on_confirm: move |_| {
                close(ctx);
                let m = model.read();
                actions::report(app, ctx, &m, ids.clone(), phishing, *block.peek());
            },
            Checkbox { checked: block(), label: format!("Also block {who}"), onchange: move |v| block.set(v) }
        }
    }
}

const IGNORE_SKIP_KEY: &str = "mail.ignore_skip_confirm";

pub fn ignore_without_confirm() -> bool {
    store().kv_get(IGNORE_SKIP_KEY).ok().flatten().as_deref() == Some("1")
}

pub fn ignore_now(app: AppState, ctx: MailCtx, conversation: &str) {
    if let Some(records) = app.check(store().ignore_conversation(conversation)) {
        let undo = crate::state::UndoAction::Moves(records);
        ctx.push_undo(undo.clone());
        app.toast_with_undo("Ignored the conversation", undo);
        app.bump_mail();
    }
}

#[component]
fn IgnoreDialog(conversation: String) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let model = use_model();
    let mut skip = use_signal(|| false);
    rsx! {
        Confirm {
            title: "Ignore this conversation?",
            body: "This conversation and any new messages in it will be moved to Deleted Items.",
            confirm: "Ignore",
            on_confirm: move |_| {
                close(ctx);
                if *skip.peek() {
                    let _ = store().kv_set(IGNORE_SKIP_KEY, "1");
                }
                let ids: Vec<MessageId> = store().conversation(&conversation).unwrap_or_default().iter().map(|m| m.id).collect();
                ignore_now(app, ctx, &conversation);
                let m = model.read();
                actions::select_after_removal(app, ctx, &m, &ids);
            },
            Checkbox { checked: skip(), label: "Don't show this again", onchange: move |v| skip.set(v) }
        }
    }
}

#[component]
fn DetailsDialog(id: MessageId) -> Element {
    let ctx = use_mail();
    let app = use_app();
    let s = store();
    let Some(m) = s.message(id).ok().flatten() else { return rsx! {} };
    let folder = s.folder(m.folder_id).ok().flatten().map(|f| f.name).unwrap_or_default();
    let addr = |l: &[EmailAddress]| l.iter().map(|a| if a.name.is_empty() { a.address.clone() } else { format!("\"{}\" <{}>", a.name, a.address) }).collect::<Vec<_>>().join(", ");
    let mut lines = vec![
        format!("From: {}", addr(std::slice::from_ref(&m.from))),
        format!("To: {}", addr(&m.to)),
    ];
    if !m.cc.is_empty() {
        lines.push(format!("Cc: {}", addr(&m.cc)));
    }
    lines.push(format!("Subject: {}", m.subject));
    lines.push(format!("Date: {}", m.sent_at.unwrap_or(m.received_at).with_timezone(&Local).to_rfc2822()));
    lines.push(format!("Received: {}", m.received_at.with_timezone(&Local).to_rfc2822()));
    if !m.internet_message_id.is_empty() {
        lines.push(format!("Message-ID: <{}>", m.internet_message_id));
    }
    if let Some(r) = &m.in_reply_to {
        lines.push(format!("In-Reply-To: <{r}>"));
    }
    lines.push(format!("Importance: {:?}", m.importance));
    if !m.categories.is_empty() {
        lines.push(format!("Keywords: {}", m.categories.join(", ")));
    }
    lines.push(format!("X-Axentra-Folder: {folder}"));
    lines.push(format!("X-Axentra-Conversation: {}", m.conversation_id));
    let text = lines.join("\n");
    let js_text = serde_json::to_string(&text).unwrap_or_default();
    rsx! {
        Dialog {
            title: "Message details",
            width: 640,
            on_close: move |_| close(ctx),
            actions: rsx! {
                Button {
                    label: "Copy",
                    icon: "copy",
                    onclick: move |_| {
                        document::eval(&format!("navigator.clipboard && navigator.clipboard.writeText({js_text})"));
                        app.toast("Copied message details");
                    },
                }
                Button { label: "Close", variant: Variant::Primary, onclick: move |_| close(ctx) }
            },
            p { class: "ax-mail-dialog-note", "Headers kept for this message. Axentra stores the parsed message, so transport headers aren't available." }
            pre { class: "ax-mail-headers ax-selectable", "{text}" }
        }
    }
}

#[component]
fn NewCategoryDialog(ids: Vec<MessageId>) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let mut name = use_signal(String::new);
    let mut color = use_signal(|| "blue".to_string());
    let mut error = use_signal(|| None::<String>);
    let ids_sig = use_signal(|| ids.clone());
    let mut save = move || {
        let ids2 = ids_sig.peek().clone();
        let n = name.peek().trim().to_string();
        let s = store();
        if n.is_empty() {
            error.set(Some("Enter a name for the category.".into()));
            return;
        }
        if n.chars().count() > 255 {
            error.set(Some("Category names can be at most 255 characters.".into()));
            return;
        }
        if s.categories().unwrap_or_default().iter().any(|c| c.name.eq_ignore_ascii_case(&n)) {
            error.set(Some("A category with this name already exists.".into()));
            return;
        }
        if app.check(s.upsert_category(&Category { name: n.clone(), color: color.peek().clone() })).is_none() {
            return;
        }
        close(ctx);
        if !ids2.is_empty() {
            actions::toggle_category(app, ctx, ids2, &n);
        }
        app.bump_mail();
    };
    rsx! {
        Dialog {
            title: "New category",
            width: 420,
            on_close: move |_| close(ctx),
            actions: rsx! {
                Button { label: "Save", variant: Variant::Primary, onclick: move |_| save() }
                Button { label: "Cancel", onclick: move |_| close(ctx) }
            },
            TextField {
                label: "Name",
                value: name(),
                autofocus: true,
                oninput: move |v| {
                    name.set(v);
                    error.set(None);
                },
                onkeydown: move |e: KeyboardEvent| {
                    ctx.mark_typing_for(&e);
                    if e.key() == Key::Enter {
                        save();
                    }
                },
            }
            if let Some(err) = error() { div { class: "ax-mail-node-error", role: "alert", "{err}" } }
            ColorGrid { value: color(), onchange: move |c| color.set(c) }
        }
    }
}

#[component]
fn ColorGrid(value: String, onchange: EventHandler<String>) -> Element {
    rsx! {
        div { class: "ax-mail-swatches", role: "radiogroup", "aria-label": "Color",
            for (name, light, _dark) in CATEGORY_COLORS.iter().copied() {
                button {
                    key: "{name}",
                    r#type: "button",
                    class: "ax-mail-swatch",
                    class: if value == name { "ax-selected" },
                    role: "radio",
                    "aria-checked": if value == name { "true" } else { "false" },
                    title: "{name}",
                    "aria-label": "{name}",
                    style: "background:{light}",
                    onclick: move |_| onchange.call(name.to_string()),
                    if value == name { Icon { name: "checkmark", size: 16 } }
                }
            }
        }
    }
}

#[component]
fn ManageCategoriesDialog() -> Element {
    let app = use_app();
    let ctx = use_mail();
    let _ = app.mail_rev();
    let cats = store().categories().unwrap_or_default();
    let mut editing = use_signal(|| None::<(String, String, String)>); // (original, name, color)
    let mut confirm_delete = use_signal(|| None::<String>);
    let mut error = use_signal(|| None::<String>);
    let mut commit = move || {
        let Some((orig, name, color)) = editing.peek().clone() else { return };
        let name = name.trim().to_string();
        let s = store();
        if name.is_empty() {
            error.set(Some("Enter a name for the category.".into()));
            return;
        }
        if !name.eq_ignore_ascii_case(&orig) && s.categories().unwrap_or_default().iter().any(|c| c.name.eq_ignore_ascii_case(&name)) {
            error.set(Some("A category with this name already exists.".into()));
            return;
        }
        let r = s.upsert_category(&Category { name: name.clone(), color }).and_then(|_| {
            if name != orig {
                s.rename_message_category(&orig, &name)?;
                s.delete_category(&orig)?;
            }
            Ok(())
        });
        if app.check(r).is_some() {
            editing.set(None);
            error.set(None);
            app.bump_mail();
        }
    };
    rsx! {
        Dialog {
            title: "Categories",
            width: 520,
            on_close: move |_| close(ctx),
            actions: rsx! {
                Button { label: "Create category", icon: "add", variant: Variant::Subtle, class: "ax-mail-dialog-left", onclick: move |_| { let mut ctx = ctx; ctx.dialog.set(Some(MailDialog::NewCategory(Vec::new()))) } }
                Button { label: "Done", variant: Variant::Primary, onclick: move |_| close(ctx) }
            },
            p { class: "ax-mail-dialog-note", "Categories are kept on this device." }
            div { class: "ax-mail-catlist",
                for c in cats {
                    {
                        let is_editing = editing().as_ref().is_some_and(|(o, _, _)| *o == c.name);
                        let (light, _) = crate::kit::category_colors(&c.color);
                        let (n1, n2, n3, c1) = (c.name.clone(), c.name.clone(), c.name.clone(), c.color.clone());
                        if is_editing {
                            let (_, ename, ecolor) = editing().unwrap_or_default();
                            rsx! {
                                div { key: "{c.name}", class: "ax-mail-catrow ax-editing",
                                    TextField {
                                        value: ename,
                                        autofocus: true,
                                        oninput: move |v: String| {
                                            if let Some(e) = editing.write().as_mut() {
                                                e.1 = v;
                                            }
                                            error.set(None);
                                        },
                                        onkeydown: move |e: KeyboardEvent| {
                                            ctx.mark_typing_for(&e);
                                            if e.key() == Key::Enter {
                                                commit();
                                            }
                                        },
                                    }
                                    ColorGrid {
                                        value: ecolor,
                                        onchange: move |col: String| {
                                            if let Some(e) = editing.write().as_mut() {
                                                e.2 = col;
                                            }
                                        },
                                    }
                                    if let Some(err) = error() { div { class: "ax-mail-node-error", role: "alert", "{err}" } }
                                    div { class: "ax-row", style: "gap:8px",
                                        Button { label: "Save", variant: Variant::Primary, small: true, onclick: move |_| commit() }
                                        Button { label: "Cancel", small: true, onclick: move |_| editing.set(None) }
                                    }
                                }
                            }
                        } else if confirm_delete().as_deref() == Some(c.name.as_str()) {
                            rsx! {
                                div { key: "{c.name}", class: "ax-mail-catrow ax-confirm", role: "alert",
                                    span { class: "ax-grow", "Delete category \"{c.name}\"? It will be removed from all items." }
                                    Button {
                                        label: "Delete",
                                        variant: Variant::Primary,
                                        small: true,
                                        onclick: move |_| {
                                            let s = store();
                                            let r = s.remove_message_category(&n3).and_then(|_| s.delete_category(&n3));
                                            if app.check(r).is_some() {
                                                confirm_delete.set(None);
                                                app.bump_mail();
                                            }
                                        },
                                    }
                                    Button { label: "Cancel", small: true, onclick: move |_| confirm_delete.set(None) }
                                }
                            }
                        } else {
                            rsx! {
                                div { key: "{c.name}", class: "ax-mail-catrow",
                                    span { class: "ax-mail-cat-swatch", style: "background:{light}" }
                                    span { class: "ax-grow ax-ellipsis", "{c.name}" }
                                    crate::kit::IconButton { icon: "edit", title: "Edit {n1}", onclick: move |_| editing.set(Some((n1.clone(), n1.clone(), c1.clone()))) }
                                    crate::kit::IconButton { icon: "delete", title: "Delete {n2}", onclick: move |_| confirm_delete.set(Some(n2.clone())) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// "Choose a date" for snooze, or "Custom" for a flag's due date.
#[component]
fn DateDialog(ids: Vec<MessageId>, snooze: bool) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let model = use_model();
    let tomorrow = Local::now().date_naive() + chrono::Duration::days(1);
    let mut date = use_signal(|| tomorrow.format("%Y-%m-%d").to_string());
    let mut time = use_signal(|| "08:00".to_string());
    let mut error = use_signal(|| None::<String>);
    let h24 = app.settings.read().use_24h_time;
    let times: Vec<(String, String)> = (0..48)
        .map(|i| {
            let t = NaiveTime::from_hms_opt(i / 2, (i % 2) * 30, 0).unwrap_or(NaiveTime::MIN);
            (t.format("%H:%M").to_string(), if h24 { t.format("%H:%M").to_string() } else { t.format("%-I:%M %p").to_string() })
        })
        .collect();
    let ids_sig = use_signal(|| ids.clone());
    let mut save = move || {
        let ids2 = ids_sig.peek().clone();
        let parsed = NaiveDate::parse_from_str(date.peek().trim(), "%Y-%m-%d").or_else(|_| NaiveDate::parse_from_str(date.peek().trim(), "%m/%d/%Y"));
        let Ok(d) = parsed else {
            error.set(Some("Enter a date like 2026-10-21.".into()));
            return;
        };
        if snooze {
            let t = NaiveTime::parse_from_str(&time.peek(), "%H:%M").unwrap_or(NaiveTime::from_hms_opt(8, 0, 0).unwrap_or(NaiveTime::MIN));
            let Some(when) = Local.from_local_datetime(&d.and_time(t)).earliest().map(|x| x.with_timezone(&Utc)) else {
                error.set(Some("That time doesn't exist in your time zone.".into()));
                return;
            };
            if when <= Utc::now() {
                error.set(Some("Choose a time in the future.".into()));
                return;
            }
            close(ctx);
            let m = model.read();
            actions::snooze(app, ctx, &m, ids2, when);
        } else {
            close(ctx);
            actions::flag(app, ctx, ids2, FlagStatus::Flagged, Some(d));
        }
    };
    rsx! {
        Dialog {
            title: if snooze { "Choose a date" } else { "Custom" },
            width: 400,
            on_close: move |_| close(ctx),
            actions: rsx! {
                Button { label: if snooze { "Snooze" } else { "Save" }, variant: Variant::Primary, onclick: move |_| save() }
                Button { label: "Cancel", onclick: move |_| close(ctx) }
            },
            p { class: "ax-mail-dialog-text", if snooze { "Bring the message back to your Inbox on:" } else { "Flag for follow-up, due on:" } }
            TextField {
                label: if snooze { "Date" } else { "Due date" },
                input_type: "date",
                value: date(),
                oninput: move |v| {
                    date.set(v);
                    error.set(None);
                },
                onkeydown: typing(ctx),
            }
            if snooze {
                Select { label: "Time", value: time(), options: times, onchange: move |v| time.set(v) }
            }
            if let Some(err) = error() { div { class: "ax-mail-node-error", role: "alert", "{err}" } }
        }
    }
}

/// "Set up Archive" for accounts without an Archive folder (docs/spec/mail.md
/// §10.2): create one, or use an existing folder. The choice is remembered
/// (the folder becomes the account's Archive), then the messages are archived.
#[component]
fn SetUpArchiveDialog(ids: Vec<MessageId>) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let model = use_model();
    let s = store();
    let msgs = s.messages_by_ids(&ids).unwrap_or_default();
    let mut accounts: Vec<AccountId> = msgs.iter().map(|m| m.account_id).collect();
    accounts.sort();
    accounts.dedup();
    let missing: Vec<AccountId> = accounts.into_iter().filter(|a| s.folder_by_role(*a, FolderRole::Archive).ok().flatten().is_none()).collect();
    let mut create = use_signal(|| true);
    // "Choose an existing folder" is offered for one account at a time.
    let choices: Vec<(String, String)> = match missing.as_slice() {
        [one] => folder_rows(Some(*one))
            .into_iter()
            .filter(|(f, _)| f.role == FolderRole::Custom && !s.snoozed_folder_ids().unwrap_or_default().contains(&f.id))
            .map(|(f, d)| (f.id.to_string(), format!("{}{}", "  ".repeat(d), f.name)))
            .collect(),
        _ => Vec::new(),
    };
    let mut chosen = use_signal(|| choices.first().map(|(id, _)| id.clone()).unwrap_or_default());
    let ids_sig = use_signal(|| ids.clone());
    let missing_sig = use_signal(|| missing.clone());
    let can_choose = !choices.is_empty();
    rsx! {
        Dialog {
            title: "Set up Archive",
            width: 448,
            on_close: move |_| close(ctx),
            actions: rsx! {
                Button {
                    label: "OK",
                    variant: Variant::Primary,
                    onclick: move |_| {
                        let s = store();
                        let result = if *create.peek() {
                            missing_sig.peek().iter().try_for_each(|a| s.ensure_role_folder(*a, FolderRole::Archive).map(|_| ()))
                        } else {
                            match chosen.peek().parse::<FolderId>() {
                                Ok(f) => s.set_folder_role(f, FolderRole::Archive),
                                Err(_) => return,
                            }
                        };
                        close(ctx);
                        if app.check(result).is_some() {
                            app.bump_mail();
                            let m = model.read();
                            actions::archive(app, ctx, &m, ids_sig.peek().clone());
                        }
                    },
                }
                Button { label: "Cancel", onclick: move |_| close(ctx) }
            },
            p { class: "ax-mail-dialog-text", "This account doesn't have an Archive folder yet. Archived messages are moved there, out of your Inbox but easy to find." }
            div { class: "ax-mail-radios", role: "radiogroup", "aria-label": "Archive folder",
                label { class: "ax-mail-radio",
                    input { r#type: "radio", name: "archive-setup", checked: create(), onchange: move |_| create.set(true) }
                    span { "Create Archive folder" }
                }
                label { class: "ax-mail-radio", class: if !can_choose { "ax-disabled" },
                    input { r#type: "radio", name: "archive-setup", checked: !create(), disabled: !can_choose, onchange: move |_| create.set(false) }
                    span { if can_choose { "Choose an existing folder…" } else { "Choose an existing folder… (no folders to choose from)" } }
                }
            }
            if !create() && can_choose {
                Select { label: "Folder", value: chosen(), options: choices, onchange: move |v| chosen.set(v) }
            }
        }
    }
}

/// Help: quick orientation plus the ways into shortcuts and settings.
#[component]
fn HelpDialog() -> Element {
    let mut app = use_app();
    let mut ctx = use_mail();
    const TIPS: &[(&str, &str)] = &[
        ("Read faster", "Press Up and Down to move through messages, Enter to open one, and Escape to go back."),
        ("Stay organized", "Right-click a message to flag, pin, categorize, snooze or move it. Drag messages onto a folder to file them."),
        ("Find anything", "Search understands from:, to:, subject:, hasattachments:yes, is:unread and received:\"last week\"."),
        ("Change the layout", "View > Reading pane puts messages on the right, at the bottom, or hides the pane. View > Density makes the list roomier or more compact."),
        ("Undo", "Most actions show an Undo button for a few seconds, and Ctrl+Z undoes the last one."),
    ];
    rsx! {
        Dialog {
            title: "Help",
            width: 520,
            on_close: move |_| close(ctx),
            actions: rsx! {
                Button { label: "Keyboard shortcuts", icon: "keyboard", class: "ax-mail-dialog-left", onclick: move |_| ctx.dialog.set(Some(MailDialog::Shortcuts)) }
                Button {
                    label: "Mail settings",
                    icon: "settings",
                    onclick: move |_| {
                        close(ctx);
                        app.settings_dialog.set(Some(crate::state::SettingsPage::Mail));
                    },
                }
                Button { label: "Close", variant: Variant::Primary, onclick: move |_| close(ctx) }
            },
            div { class: "ax-mail-help",
                for (title, body) in TIPS.iter() {
                    div { key: "{title}", class: "ax-mail-help-tip",
                        div { class: "ax-mail-help-title", "{title}" }
                        div { class: "ax-mail-help-body", "{body}" }
                    }
                }
            }
            p { class: "ax-mail-dialog-note", "{crate::APP_NAME} keeps your mail on this device. Pin, snooze, sweep and ignore rules don't sync with other Outlook apps." }
        }
    }
}

/// Mail keyboard shortcuts (keyboard spec §4, "Outlook" set).
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Mail",
        &[
            ("New message", "Ctrl+N"),
            ("Reply", "Ctrl+R"),
            ("Reply all", "Ctrl+Shift+R"),
            ("Forward", "Ctrl+F"),
            ("Forward as attachment", "Ctrl+Alt+F"),
            ("Delete", "Delete"),
            ("Permanently delete", "Shift+Delete"),
            ("Archive", "Backspace"),
            ("Mark as read", "Ctrl+Q"),
            ("Mark as unread", "Ctrl+U"),
            ("Flag / mark complete", "Insert"),
            ("Move to folder", "Ctrl+Shift+V"),
            ("Copy to folder", "Ctrl+Shift+Y"),
            ("Undo", "Ctrl+Z"),
            ("Print", "Ctrl+P"),
            ("Go to Inbox", "Ctrl+Shift+I"),
            ("Go to the folder pane", "Ctrl+Y"),
            ("Next / previous region", "F6 / Shift+F6"),
            ("Show keyboard shortcuts", "?"),
        ],
    ),
    (
        "Message list",
        &[
            ("Next / previous item", "Down / Up"),
            ("Next / previous message", "Ctrl+. / Ctrl+,"),
            ("Extend the selection", "Shift+Down / Shift+Up"),
            ("First / last item", "Home / End"),
            ("Select all", "Ctrl+A"),
            ("Toggle selection", "Space"),
            ("Expand / collapse a conversation", "Right / Left"),
            ("Open the item", "Enter"),
            ("Open in new window", "Shift+Enter"),
            ("Context menu", "Shift+F10"),
            ("Clear the selection / cancel search", "Esc"),
        ],
    ),
    (
        "Folder pane",
        &[
            ("Previous / next folder", "Up / Down"),
            ("Expand / collapse", "Right / Left"),
            ("Open folder", "Enter"),
            ("Rename folder", "F2"),
            ("New subfolder", "Ctrl+Shift+E"),
            ("Jump to a folder", "Type its name"),
        ],
    ),
];

#[component]
fn ShortcutsDialog() -> Element {
    let ctx = use_mail();
    let app = use_app();
    let mut q = use_signal(String::new);
    let query = q().to_lowercase();
    let off = app.settings.read().keyboard_mode == KeyboardMode::Off;
    let found: usize = SHORTCUTS
        .iter()
        .map(|(_, rows)| rows.iter().filter(|(a, k)| query.is_empty() || a.to_lowercase().contains(&query) || k.to_lowercase().contains(&query)).count())
        .sum();
    rsx! {
        Dialog {
            title: "Keyboard shortcuts",
            width: 560,
            on_close: move |_| close(ctx),
            p { class: "ax-mail-dialog-note", if off { "Keyboard shortcuts are turned off. Navigation keys and Delete still work." } else { "Shortcut set: Outlook" } }
            TextField { value: q(), placeholder: "Search shortcuts", autofocus: true, oninput: move |v| q.set(v), onkeydown: typing(ctx) }
            span { class: "ax-visually-hidden", role: "status", "{found} shortcuts found" }
            div { class: "ax-mail-shortcuts",
                for (section, rows) in SHORTCUTS.iter() {
                    {
                        let rows: Vec<(&str, &str)> = rows.iter().copied().filter(|(a, k)| query.is_empty() || a.to_lowercase().contains(&query) || k.to_lowercase().contains(&query)).collect();
                        if rows.is_empty() {
                            rsx! {}
                        } else {
                            rsx! {
                                h3 { key: "{section}", class: "ax-mail-shortcuts-section", "{section}" }
                                for (action, keys) in rows {
                                    div { key: "{section}-{action}", class: "ax-mail-shortcut", "aria-label": "{action}: {keys}",
                                        span { class: "ax-grow", "{action}" }
                                        span { class: "ax-mail-keys",
                                            for k in keys.split(" / ") {
                                                kbd { class: "ax-mail-key", "{k}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
