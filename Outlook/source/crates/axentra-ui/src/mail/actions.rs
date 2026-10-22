//! Mail commands. The ribbon, context menus, hover actions, the reading
//! pane and keyboard shortcuts all dispatch command ids through [`run`], so
//! a command behaves the same wherever it comes from.

use std::collections::HashSet;

use axentra_core::*;
use chrono::{Duration, Local, Timelike, Utc};
use dioxus::prelude::*;

use super::ctx::{FolderMemory, MailCtx, MailDialog, Open, PersonView, RowKey};
use super::dates::{self, FlagPreset, SnoozePreset};
use super::model::ListModel;
use crate::state::{AppState, ComposeRequest, NavRequest, SearchState, UndoAction, store};

pub fn items(n: usize) -> String {
    if n == 1 { "1 item".into() } else { format!("{n} items") }
}

/// Messages a command acts on: the selection, else the focused row, else the
/// open message.
pub fn targets(ctx: MailCtx, model: &ListModel) -> Vec<MessageId> {
    let sel = ctx.selection.read().clone();
    if !sel.is_empty() {
        return model.ids_for(&sel);
    }
    if let Some(k) = ctx.focus.read().clone() {
        return model.ids_for(&[k]);
    }
    ctx.open.read().as_ref().map(|o| vec![o.message]).unwrap_or_default()
}

/// The single message a reply / forward / details command uses.
pub fn primary_message(ctx: MailCtx, model: &ListModel) -> Option<Message> {
    let sel = ctx.selection.read().clone();
    match sel.as_slice() {
        [one] => model.message(one).or_else(|| ctx.open.read().as_ref().and_then(|o| store().message(o.message).ok().flatten())),
        [] => ctx.open.read().as_ref().and_then(|o| store().message(o.message).ok().flatten()),
        _ => None,
    }
}

fn messages(ids: &[MessageId]) -> Vec<Message> {
    store().messages_by_ids(ids).unwrap_or_default()
}

/// After rows disappear (delete, move, archive...): open the next item, the
/// way Outlook does by default ("Open the next item").
pub fn select_after_removal(app: AppState, mut ctx: MailCtx, model: &ListModel, removed: &[MessageId]) {
    let removed: HashSet<MessageId> = removed.iter().copied().collect();
    let gone = |k: &RowKey| model.ids.get(k).is_some_and(|ids| !ids.is_empty() && ids.iter().all(|i| removed.contains(i)));
    let sel = ctx.selection.peek().clone();
    let open_gone = ctx.open.peek().as_ref().is_some_and(|o| removed.contains(&o.message));
    let any_selected_gone = sel.iter().any(gone);
    if !open_gone && !any_selected_gone {
        return;
    }
    let first = sel
        .iter()
        .chain(ctx.focus.peek().iter())
        .filter_map(|k| model.index_of(k))
        .min()
        .unwrap_or(0);
    let next = model.keys[first.min(model.keys.len())..]
        .iter()
        .find(|k| !gone(k))
        .or_else(|| model.keys[..first.min(model.keys.len())].iter().rev().find(|k| !gone(k)))
        .cloned();
    match next {
        Some(k) if open_gone || sel.len() == 1 => {
            ctx.selection.set(vec![k.clone()]);
            ctx.anchor.set(Some(k.clone()));
            ctx.focus.set(Some(k.clone()));
            open_row(app, ctx, model, &k);
            ctx.reveal(k);
        }
        Some(k) => {
            ctx.selection.set(Vec::new());
            ctx.focus.set(Some(k));
            ctx.open.set(None);
        }
        None => ctx.clear_selection(),
    }
}

/// Show a row in the reading pane (drafts open in compose).
pub fn open_row(mut app: AppState, mut ctx: MailCtx, model: &ListModel, key: &RowKey) {
    let Some(open) = model.opens.get(key).cloned() else { return };
    if let Some(m) = model.message(key)
        && m.is_draft
    {
        ctx.open.set(None);
        app.compose(ComposeRequest { draft: Some(m.id), ..ComposeRequest::new_message() });
        return;
    }
    if app.compose.peek().as_ref().is_some_and(|c| !c.popout) {
        app.compose.set(None);
    }
    ctx.open.set(Some(open));
}

fn toast_moves(app: AppState, text: String, records: Vec<MoveRecord>) {
    if records.is_empty() {
        app.toast(text);
    } else {
        app.toast_with_undo(text, UndoAction::Moves(records));
    }
}

/// Snapshot messages before a change so it can be undone (with its undo
/// bar, or `app.push_undo` when the change shows none).
fn snapshot(ids: &[MessageId]) -> UndoAction {
    UndoAction::RestoreMessages(messages(ids))
}

fn folder_name(id: FolderId) -> String {
    store().folder(id).ok().flatten().map(|f| f.name).unwrap_or_else(|| "folder".into())
}

pub fn move_to(app: AppState, ctx: MailCtx, model: &ListModel, ids: Vec<MessageId>, folder: FolderId) {
    let s = store();
    let Some(records) = app.check(s.move_messages(&ids, folder)) else { return };
    let _ = s.record_recent_folder(folder);
    let name = folder_name(folder);
    let text = if records.len() == 1 { format!("Moved to {name}") } else { format!("Moved {} to {name}", items(records.len())) };
    select_after_removal(app, ctx, model, &ids);
    toast_moves(app, text, records);
    app.bump_mail();
}

pub fn copy_to(app: AppState, ids: Vec<MessageId>, folder: FolderId) {
    let s = store();
    if let Some(copies) = app.check(s.copy_messages(&ids, folder)) {
        let _ = s.record_recent_folder(folder);
        app.toast(format!("Copied {} to {}", items(copies.len()), folder_name(folder)));
        app.bump_mail();
    }
}

pub fn delete(app: AppState, mut ctx: MailCtx, model: &ListModel, ids: Vec<MessageId>) {
    if ids.is_empty() {
        return;
    }
    let s = store();
    // Items already in Deleted Items are removed for good: confirm first.
    let deleted_folders: HashSet<FolderId> =
        s.folders(None).unwrap_or_default().into_iter().filter(|f| f.role == FolderRole::Deleted).map(|f| f.id).collect();
    let in_deleted = messages(&ids).iter().any(|m| deleted_folders.contains(&m.folder_id));
    if in_deleted {
        ctx.dialog.set(Some(MailDialog::PermanentDelete(ids)));
        return;
    }
    let Some(records) = app.check(s.delete_messages(&ids)) else { return };
    select_after_removal(app, ctx, model, &ids);
    toast_moves(app, format!("Deleted {}", items(records.len())), records);
    app.bump_mail();
}

pub fn delete_permanently(app: AppState, ctx: MailCtx, model: &ListModel, ids: Vec<MessageId>) {
    let n = ids.len();
    if app.check(store().delete_messages_permanently(&ids)).is_some() {
        select_after_removal(app, ctx, model, &ids);
        app.toast(format!("Permanently deleted {}", items(n)));
        app.bump_mail();
    }
}

pub fn archive(app: AppState, mut ctx: MailCtx, model: &ListModel, ids: Vec<MessageId>) {
    let s = store();
    let accounts: HashSet<AccountId> = messages(&ids).iter().map(|m| m.account_id).collect();
    let with_archive: HashSet<AccountId> =
        s.folders(None).unwrap_or_default().into_iter().filter(|f| f.role == FolderRole::Archive).map(|f| f.account_id).collect();
    if !accounts.is_subset(&with_archive) {
        // IMAP accounts may have no Archive folder: set one up first (§10.2).
        ctx.dialog.set(Some(MailDialog::SetUpArchive(ids)));
        return;
    }
    let Some(records) = app.check(s.archive_messages(&ids)) else { return };
    select_after_removal(app, ctx, model, &ids);
    toast_moves(app, format!("Archived {}", items(records.len())), records);
    app.bump_mail();
}

pub fn report(app: AppState, ctx: MailCtx, model: &ListModel, ids: Vec<MessageId>, phishing: bool, block: bool) {
    let s = store();
    if block {
        for m in messages(&ids) {
            let _ = s.block_sender(&m.from);
        }
    }
    let result = if phishing {
        s.delete_messages(&ids)
    } else {
        s.mark_junk(&ids)
    };
    let Some(records) = app.check(result) else { return };
    select_after_removal(app, ctx, model, &ids);
    let text = if phishing { "Reported as phishing" } else { "Reported as junk" };
    toast_moves(app, text.into(), records);
    app.bump_mail();
}

pub fn restore(app: AppState, ctx: MailCtx, model: &ListModel, ids: Vec<MessageId>) {
    let Some(records) = app.check(store().restore_messages(&ids)) else { return };
    let text = match records.as_slice() {
        [one] => store()
            .message(one.message_id)
            .ok()
            .flatten()
            .map(|m| format!("Restored to {}", folder_name(m.folder_id)))
            .unwrap_or_else(|| "Restored".into()),
        many => format!("Restored {}", items(many.len())),
    };
    select_after_removal(app, ctx, model, &ids);
    toast_moves(app, text, records);
    app.bump_mail();
}

pub fn set_read(app: AppState, ids: Vec<MessageId>, read: bool) {
    if ids.is_empty() {
        return;
    }
    let undo = snapshot(&ids);
    if app.check(store().set_read(&ids, read)).is_some() {
        if ids.len() > 1 {
            app.toast_with_undo(if read { "Marked as read" } else { "Marked as unread" }, undo);
        } else {
            app.push_undo(undo);
        }
        app.bump_mail();
    }
}

pub fn set_pinned(app: AppState, ids: Vec<MessageId>, pinned: bool) {
    let undo = snapshot(&ids);
    if app.check(store().set_pinned(&ids, pinned)).is_some() {
        app.toast_with_undo(if pinned { "Pinned" } else { "Unpinned" }, undo);
        app.bump_mail();
    }
}

pub fn flag(app: AppState, ids: Vec<MessageId>, status: FlagStatus, due: Option<chrono::NaiveDate>) {
    let undo = snapshot(&ids);
    if app.check(store().set_flag(&ids, status, due)).is_some() {
        app.push_undo(undo);
        app.bump_mail();
        app.bump_tasks();
    }
}

fn flag_preset(app: AppState, ids: Vec<MessageId>, preset: FlagPreset) {
    let s = app.settings.peek().clone();
    let due = dates::flag_due(preset, Local::now().date_naive(), s.week_start, &s.work_days);
    flag(app, ids, FlagStatus::Flagged, due);
}

pub fn snooze(app: AppState, ctx: MailCtx, model: &ListModel, ids: Vec<MessageId>, until: chrono::DateTime<Utc>) {
    let undo = snapshot(&ids);
    if app.check(store().snooze(&ids, Some(until))).is_some() {
        select_after_removal(app, ctx, model, &ids);
        let when = dates::short_when(until, Local::now(), app.settings.peek().use_24h_time);
        app.toast_with_undo(format!("Snoozed until {when}"), undo);
        app.bump_mail();
    }
}

pub fn toggle_category(app: AppState, ids: Vec<MessageId>, name: &str) {
    let undo = snapshot(&ids);
    if app.check(store().toggle_category(&ids, name)).is_some() {
        app.push_undo(undo);
        app.bump_mail();
    }
}

pub fn set_inference(app: AppState, ctx: MailCtx, model: &ListModel, ids: Vec<MessageId>, class: InferenceClass, always: bool) {
    let s = store();
    let undo = snapshot(&ids);
    let mut all = ids.clone();
    if always {
        let senders: HashSet<String> = messages(&ids).into_iter().map(|m| m.from.address.to_lowercase()).collect();
        for a in &senders {
            let _ = s.set_sender_inference(a, class);
        }
        // Existing Inbox mail from those senders follows too.
        if let Some(f) = model.folder.as_ref() {
            for m in s.messages(&MessageQuery { folder_id: Some(f.id), ..Default::default() }).unwrap_or_default() {
                if senders.contains(&m.from.address.to_lowercase()) && !all.contains(&m.id) {
                    all.push(m.id);
                }
            }
        }
    }
    if app.check(s.set_inference(&all, class)).is_some() {
        select_after_removal(app, ctx, model, &all);
        let target = if class == InferenceClass::Other { "Other" } else { "Focused" };
        let text = if always { format!("Messages from this sender will always go to {target}") } else { format!("Moved to {target}") };
        app.toast_with_undo(text, undo);
        app.bump_mail();
    }
}

pub fn compose(app: AppState, kind: ComposeKind, id: MessageId) {
    app.compose(ComposeRequest::reply(kind, id));
}

pub fn save_as(app: AppState, id: MessageId) {
    let s = store();
    let Some(m) = app.check(s.message(id)).flatten() else { return };
    let Some(eml) = app.check(s.message_as_eml(id)) else { return };
    let subject = if m.subject.trim().is_empty() { "(No subject)".to_string() } else { m.subject.clone() };
    match super::files::save_to_downloads(&format!("{subject}.eml"), eml.as_bytes()) {
        Ok(p) => {
            app.toast(format!("Saved {} to Downloads", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        }
        Err(e) => {
            app.error(format!("Couldn't save the message: {e}"));
        }
    }
}

/// Print the open message: the read window when one is open, else the
/// reading pane (nothing else on the page is printed).
pub fn print() {
    document::eval(
        "const p = document.querySelector('.ax-mail-read-window') || document.querySelector('.ax-mail-reading');
         if (p) { p.classList.add('ax-mail-print-target'); window.print(); p.classList.remove('ax-mail-print-target'); }",
    );
}

/// "Open in new window" (Shift+Enter): the conversation in a read window
/// over the app; drafts open in a compose window.
pub fn open_window(app: AppState, mut ctx: MailCtx, m: &Message) {
    if m.is_draft {
        app.compose(ComposeRequest { draft: Some(m.id), popout: true, ..ComposeRequest::new_message() });
        return;
    }
    let settings = app.settings.peek().clone();
    ctx.popout.set(Some(Open { message: m.id, conversation: settings.conversation_view.then(|| m.conversation_id.clone()), mark: vec![m.id] }));
    if settings.mark_as_read != MarkAsRead::Never && !m.is_read && app.check(store().set_read(&[m.id], true)).is_some() {
        app.bump_mail();
    }
}

/// Reply / reply all / forward. From the read window the reply opens in a
/// compose window in its place, as it does in Outlook's message window.
pub fn respond(app: AppState, mut ctx: MailCtx, kind: ComposeKind, id: MessageId, from_window: bool) {
    if from_window {
        ctx.popout.set(None);
        app.compose(ComposeRequest { popout: true, ..ComposeRequest::reply(kind, id) });
    } else {
        compose(app, kind, id);
    }
}

pub fn exit_search(mut app: AppState) {
    let serial = app.search.peek().serial + 1;
    app.search.set(SearchState { serial, ..Default::default() });
}

/// Switch the list to a folder (leaving search). The selection, open item,
/// Focused/Other tab and scroll position of the folder being left are kept
/// for the session and restored when the user comes back (§3.14).
pub fn go_to_folder(app: AppState, mut ctx: MailCtx, folder: FolderId) {
    if app.search.peek().active {
        exit_search(app);
    }
    let current = *ctx.folder.peek();
    let from_person = ctx.person.peek().is_some();
    if from_person {
        ctx.person.set(None);
    } else if current == Some(folder) {
        return;
    }
    if let Some(cur) = current.filter(|_| !from_person) {
        let memory = FolderMemory {
            selection: ctx.selection.peek().clone(),
            anchor: ctx.anchor.peek().clone(),
            focus: ctx.focus.peek().clone(),
            open: ctx.open.peek().clone(),
            tab: *ctx.tab.peek(),
            scroll: *ctx.list_scroll.peek(),
        };
        ctx.memory.write().insert(cur, memory);
    }
    let memory = ctx.memory.peek().get(&folder).cloned().map(|m| still_in_folder(m, folder)).unwrap_or_default();
    ctx.folder.set(Some(folder));
    ctx.selection.set(memory.selection);
    ctx.anchor.set(memory.anchor);
    ctx.focus.set(memory.focus);
    ctx.open.set(memory.open);
    ctx.tab.set(memory.tab);
    ctx.expanded.set(None);
    ctx.select_mode.set(false);
    ctx.full_view.set(false);
}

/// Show a favourite person's mail in the list (Favorites, §2.3).
pub fn show_person(app: AppState, mut ctx: MailCtx, person: PersonView) {
    if app.search.peek().active {
        exit_search(app);
    }
    if ctx.person.peek().as_ref() == Some(&person) {
        return;
    }
    if let Some(cur) = *ctx.folder.peek()
        && ctx.person.peek().is_none()
    {
        let memory = FolderMemory {
            selection: ctx.selection.peek().clone(),
            anchor: ctx.anchor.peek().clone(),
            focus: ctx.focus.peek().clone(),
            open: ctx.open.peek().clone(),
            tab: *ctx.tab.peek(),
            scroll: *ctx.list_scroll.peek(),
        };
        ctx.memory.write().insert(cur, memory);
    }
    ctx.person.set(Some(person));
    ctx.selection.set(Vec::new());
    ctx.anchor.set(None);
    ctx.focus.set(None);
    ctx.open.set(None);
    ctx.expanded.set(None);
    ctx.select_mode.set(false);
    ctx.full_view.set(false);
}

/// Drop remembered rows whose messages left the folder meanwhile.
fn still_in_folder(mut m: FolderMemory, folder: FolderId) -> FolderMemory {
    let s = store();
    let msg_ids: Vec<MessageId> = m
        .selection
        .iter()
        .chain(m.focus.iter())
        .chain(m.anchor.iter())
        .filter_map(|k| if let RowKey::Msg(id) = k { Some(*id) } else { None })
        .chain(m.open.iter().map(|o| o.message))
        .collect();
    let here: HashSet<MessageId> = s.messages_by_ids(&msg_ids).unwrap_or_default().into_iter().filter(|x| x.folder_id == folder).map(|x| x.id).collect();
    let convs: HashSet<String> = m
        .selection
        .iter()
        .filter_map(|k| if let RowKey::Conv(c) = k { Some(c.clone()) } else { None })
        .filter(|c| s.conversation(c).unwrap_or_default().iter().any(|x| x.folder_id == folder))
        .collect();
    let keep = |k: &RowKey| match k {
        RowKey::Msg(id) => here.contains(id),
        RowKey::Conv(c) => convs.contains(c),
    };
    m.selection.retain(keep);
    m.anchor = m.anchor.filter(keep);
    m.focus = m.focus.filter(keep);
    m.open = m.open.filter(|o| here.contains(&o.message) || o.conversation.as_ref().is_some_and(|c| convs.contains(c)));
    if m.selection.len() != 1 {
        m.open = None;
    }
    m
}

/// Run a command id on `ids` (see the module docs). Returns false for
/// unknown ids.
pub fn run(mut app: AppState, mut ctx: MailCtx, model: &ListModel, cmd: &str, ids: Vec<MessageId>) -> bool {
    let s = store();
    let first = ids.first().copied();
    let single = || (ids.len() == 1).then(|| ids[0]);
    match cmd {
        "new_mail" => app.compose(ComposeRequest::new_message()),
        "new_event" => {
            // The next half hour, like Outlook's New event.
            let now = Local::now();
            let add = if now.minute() < 30 { 30 - now.minute() } else { 60 - now.minute() };
            let start = (now + Duration::minutes(add as i64)).with_second(0).and_then(|t| t.with_nanosecond(0)).unwrap_or(now);
            let start = start.with_timezone(&Utc);
            app.navigate(NavRequest::NewEvent { start, end: start + Duration::minutes(30), all_day: false });
        }
        "reply" | "reply_all" | "forward" | "forward_attachment" => {
            // With the read window open, its message is the one answered.
            let window = ctx.popout.peek().as_ref().map(|o| o.message);
            let id = window.or_else(|| primary_message(ctx, model).map(|m| m.id)).or(first);
            if let Some(id) = id {
                let kind = match cmd {
                    "reply" => ComposeKind::Reply,
                    "reply_all" => ComposeKind::ReplyAll,
                    _ => ComposeKind::Forward,
                };
                respond(app, ctx, kind, id, window.is_some());
            }
        }
        "edit_draft" => {
            if let Some(id) = first {
                app.compose(ComposeRequest { draft: Some(id), ..ComposeRequest::new_message() });
            }
        }
        "delete" => delete(app, ctx, model, ids),
        "delete_permanent" => {
            if !ids.is_empty() {
                ctx.dialog.set(Some(MailDialog::PermanentDelete(ids)));
            }
        }
        "archive" => archive(app, ctx, model, ids),
        "report_junk" | "report_phishing" => {
            if !ids.is_empty() {
                ctx.dialog.set(Some(MailDialog::Report { ids, phishing: cmd == "report_phishing" }));
            }
        }
        "not_junk" | "restore" => restore(app, ctx, model, ids),
        "sweep" => {
            if let Some(id) = first {
                ctx.dialog.set(Some(MailDialog::Sweep(id)));
            }
        }
        "move_picker" => ctx.dialog.set(Some(MailDialog::MoveTo { ids, copy: false })),
        "copy_picker" => ctx.dialog.set(Some(MailDialog::MoveTo { ids, copy: true })),
        "move_focused" => set_inference(app, ctx, model, ids, InferenceClass::Focused, false),
        "move_other" => set_inference(app, ctx, model, ids, InferenceClass::Other, false),
        "always_focused" => set_inference(app, ctx, model, ids, InferenceClass::Focused, true),
        "always_other" => set_inference(app, ctx, model, ids, InferenceClass::Other, true),
        "read" => set_read(app, ids, true),
        "unread" => set_read(app, ids, false),
        "toggle_read" => {
            let all_read = messages(&ids).iter().all(|m| m.is_read);
            set_read(app, ids, !all_read);
        }
        "pin" => set_pinned(app, ids, true),
        "unpin" => set_pinned(app, ids, false),
        "toggle_pin" => {
            let all_pinned = messages(&ids).iter().all(|m| m.is_pinned);
            set_pinned(app, ids, !all_pinned);
        }
        // Quick flag: flags for today, or clears a flagged item.
        "flag_toggle" => {
            if messages(&ids).iter().all(|m| m.flag == FlagStatus::Flagged) {
                flag(app, ids, FlagStatus::NotFlagged, None);
            } else {
                flag_preset(app, ids, FlagPreset::Today);
            }
        }
        // Insert: flags, or marks a flagged item complete.
        "flag_insert" => {
            if messages(&ids).iter().all(|m| m.flag == FlagStatus::Flagged) {
                flag(app, ids, FlagStatus::Complete, None);
            } else {
                flag_preset(app, ids, FlagPreset::Today);
            }
        }
        "flag_complete" => flag(app, ids, FlagStatus::Complete, None),
        "flag_clear" => flag(app, ids, FlagStatus::NotFlagged, None),
        "flag_custom" => ctx.dialog.set(Some(MailDialog::FlagCustom(ids))),
        "cat_new" => ctx.dialog.set(Some(MailDialog::NewCategory(ids))),
        "cat_manage" => ctx.dialog.set(Some(MailDialog::ManageCategories)),
        "cat_clear" => {
            let undo = snapshot(&ids);
            if app.check(s.set_categories(&ids, &[])).is_some() {
                app.push_undo(undo);
                app.bump_mail();
            }
        }
        "snooze_custom" => ctx.dialog.set(Some(MailDialog::SnoozeUntil(ids))),
        "unsnooze" => {
            let undo = snapshot(&ids);
            if app.check(s.snooze(&ids, None)).is_some() {
                select_after_removal(app, ctx, model, &ids);
                app.toast_with_undo("Unsnoozed", undo);
                app.bump_mail();
            }
        }
        "ignore" => {
            if let Some(m) = first.and_then(|id| s.message(id).ok().flatten()) {
                if s.is_conversation_ignored(&m.conversation_id).unwrap_or(false) {
                    if let Some(records) = app.check(s.stop_ignoring_conversation(&m.conversation_id)) {
                        toast_moves(app, "Stopped ignoring the conversation".into(), records);
                        app.bump_mail();
                    }
                } else if super::dialogs::ignore_without_confirm() {
                    let conv_ids: Vec<MessageId> = s.conversation(&m.conversation_id).unwrap_or_default().iter().map(|x| x.id).collect();
                    super::dialogs::ignore_now(app, &m.conversation_id);
                    select_after_removal(app, ctx, model, &conv_ids);
                } else {
                    ctx.dialog.set(Some(MailDialog::Ignore(m.conversation_id)));
                }
            }
        }
        "block" => {
            if let Some(id) = first {
                ctx.dialog.set(Some(MailDialog::Block(id)));
            }
        }
        "never_block" => {
            for m in messages(&ids) {
                let _ = s.add_safe_sender(&m.from.address);
            }
            app.toast("Added to safe senders");
        }
        "details" => {
            if let Some(id) = primary_message(ctx, model).map(|m| m.id).or(first) {
                ctx.dialog.set(Some(MailDialog::Details(id)));
            }
        }
        "print" => print(),
        "open_window" => {
            if let Some(m) = primary_message(ctx, model).or_else(|| first.and_then(|id| s.message(id).ok().flatten())) {
                open_window(app, ctx, &m);
            }
        }
        "save_as" => {
            if let Some(id) = single().or(first) {
                save_as(app, id);
            }
        }
        "select_all" => {
            ctx.selection.set(model.keys.clone());
            ctx.open.set(None);
        }
        "deselect" => ctx.clear_selection(),
        "shortcuts" => ctx.dialog.set(Some(MailDialog::Shortcuts)),
        "help" => ctx.dialog.set(Some(MailDialog::Help)),
        "view_settings" => app.settings_dialog.set(Some(crate::state::SettingsPage::Mail)),
        "manage_accounts" => app.settings_dialog.set(Some(crate::state::SettingsPage::Accounts)),
        // One stack for the whole app: the most recent action goes first,
        // a send included (compose::send_queue::undo).
        "undo" => super::compose::send_queue::undo(app, false),
        other => {
            if let Some(f) = other.strip_prefix("move:").and_then(|v| v.parse().ok()) {
                move_to(app, ctx, model, ids, f);
            } else if let Some(f) = other.strip_prefix("copy:").and_then(|v| v.parse().ok()) {
                copy_to(app, ids, f);
            } else if let Some(name) = other.strip_prefix("cat:") {
                toggle_category(app, ids, name);
            } else if let Some(p) = other.strip_prefix("flag:").and_then(FlagPreset::parse) {
                flag_preset(app, ids, p);
            } else if let Some(p) = other.strip_prefix("snooze:").and_then(SnoozePreset::parse) {
                if let Some(until) = dates::snooze_until(p, Local::now()) {
                    snooze(app, ctx, model, ids, until);
                }
            } else {
                return false;
            }
        }
    }
    true
}

/// The open item for a single message (used by navigation requests).
pub fn open_message(mut ctx: MailCtx, m: &Message, conversation_view: bool) {
    let key = RowKey::Msg(m.id);
    ctx.open.set(Some(Open {
        message: m.id,
        conversation: conversation_view.then(|| m.conversation_id.clone()),
        mark: vec![m.id],
    }));
    ctx.selection.set(vec![key.clone()]);
    ctx.anchor.set(Some(key.clone()));
    ctx.focus.set(Some(key));
}
