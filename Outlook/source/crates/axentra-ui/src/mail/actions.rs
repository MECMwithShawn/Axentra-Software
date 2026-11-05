//! Mail commands. The ribbon, context menus, hover actions, the reading
//! pane and keyboard shortcuts all dispatch command ids through [`run`], so
//! a command behaves the same wherever it comes from.

use std::collections::{HashMap, HashSet};

use axentra_core::*;
use chrono::{Duration, Local, Timelike, Utc};
use dioxus::prelude::*;

use super::ctx::{FolderMemory, MailCtx, MailDialog, Open, PersonView, RowKey, SearchScope, TopicScopeChoice};
use super::dates::{self, FlagPreset, SnoozePreset};
use super::model::ListModel;
use crate::state::{AppState, ComposeRequest, MailView, NavRequest, SearchState, UndoAction, store};

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

/// Report junk / Report phishing: the user's confirmed label, the move to
/// Junk Email and (when asked) a block rule for each sender in that account,
/// recorded as one change (all or nothing) and undone together. Phishing
/// reports stay in Junk Email with their warning; nothing is deleted or sent
/// anywhere.
pub fn report(app: AppState, ctx: MailCtx, model: &ListModel, ids: Vec<MessageId>, phishing: bool, block: bool) {
    let label = if phishing { junk::Label::Phishing } else { junk::Label::Spam };
    let Some(op) = app.check(store().report_junk(&ids, label, block)) else { return };
    select_after_removal(app, ctx, model, &ids);
    let text = if phishing { "Reported as phishing" } else { "Reported as junk" };
    app.toast_with_undo(text, UndoAction::Junk { events: op.events, rules: op.rules, moves: op.moves });
    app.bump_mail();
}

/// "Not junk": a confirmed label, then the messages go back where they came
/// from. Only this records a label; a plain Restore from Deleted Items
/// doesn't.
pub fn not_junk(app: AppState, ctx: MailCtx, model: &ListModel, ids: Vec<MessageId>) {
    let s = store();
    let Some(op) = app.check(s.mark_not_junk(&ids)) else { return };
    let (events, moves) = (op.events, op.moves);
    let in_junk: Vec<MessageId> = moves.iter().map(|m| m.message_id).collect();
    select_after_removal(app, ctx, model, &in_junk);
    let text = match moves.as_slice() {
        [] => "Marked as not junk".to_string(),
        [one] => s.message(one.message_id).ok().flatten().map(|m| format!("Not junk. Moved to {}", folder_name(m.folder_id))).unwrap_or_else(|| "Not junk".into()),
        many => format!("Not junk. Moved {}", items(many.len())),
    };
    app.toast_with_undo(text, UndoAction::Junk { events, rules: Vec::new(), moves });
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

/// The list's selection, open item, tab and scroll position, to restore later.
fn remember(ctx: MailCtx) -> FolderMemory {
    FolderMemory {
        selection: ctx.selection.peek().clone(),
        anchor: ctx.anchor.peek().clone(),
        focus: ctx.focus.peek().clone(),
        open: ctx.open.peek().clone(),
        tab: *ctx.tab.peek(),
        scroll: *ctx.list_scroll.peek(),
    }
}

/// With the reading pane hidden a message being written takes the list's
/// place, so choosing any list closes it to show that list. It saves itself
/// on the way (`save_on_drop`). Beside the list it stays where it is.
fn close_compose_over_list(app: AppState) {
    let inline = app.compose.peek().as_ref().is_some_and(|c| !c.popout);
    if inline && app.settings.peek().reading_pane_shown() == ReadingPanePosition::Hidden {
        let mut compose = app.compose;
        compose.set(None);
    }
}

/// Switch the list to a folder (leaving search). The selection, open item,
/// Focused/Other tab and scroll position of the folder being left are kept
/// for the session and restored when the user comes back (§3.14).
pub fn go_to_folder(app: AppState, mut ctx: MailCtx, folder: FolderId) {
    close_compose_over_list(app);
    // During a search the list holds the results; the folder's own list
    // was kept when the search started (`search_started`).
    let searching = app.search.peek().active;
    if searching {
        exit_search(app);
    }
    let current = *ctx.folder.peek();
    // A person's mail, Packages or a topic list: back to a folder, even the
    // one shown before it.
    let from_virtual = ctx.person.peek().is_some() || ctx.view.peek().is_some();
    if from_virtual {
        ctx.person.set(None);
        ctx.view.set(None);
    } else if current == Some(folder) {
        // Already there, maybe under an email opened full width (reading
        // pane hidden, or the desktop view): back to the list.
        if *ctx.full_view.peek() {
            ctx.open.set(None);
            ctx.full_view.set(false);
            ctx.request_list_focus();
        }
        return;
    }
    if let Some(cur) = current.filter(|_| !from_virtual && !searching) {
        let memory = remember(ctx);
        ctx.memory.write().insert(cur, memory);
    }
    let memory = ctx.memory.peek().get(&folder).cloned().map(|m| still_in_folder(m, folder)).unwrap_or_default();
    // Where the list will scroll to, also when it is shown anew (from
    // Packages, which replaces it).
    ctx.list_scroll.set(memory.scroll);
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
    close_compose_over_list(app);
    let searching = app.search.peek().active;
    if searching {
        exit_search(app);
    }
    if ctx.person.peek().as_ref() == Some(&person) {
        return;
    }
    if let Some(cur) = *ctx.folder.peek()
        && !searching
        && ctx.person.peek().is_none()
        && ctx.view.peek().is_none()
    {
        let memory = remember(ctx);
        ctx.memory.write().insert(cur, memory);
    }
    ctx.view.set(None);
    ctx.person.set(Some(person));
    ctx.selection.set(Vec::new());
    ctx.anchor.set(None);
    ctx.focus.set(None);
    ctx.open.set(None);
    ctx.expanded.set(None);
    ctx.select_mode.set(false);
    ctx.full_view.set(false);
}

/// Whether a view's feature is switched on (Settings > Mail > Packages / Topics).
pub fn view_enabled(settings: &Settings, v: MailView) -> bool {
    match v {
        MailView::Packages => settings.order_tracking,
        MailView::Topics(_) => settings.topics_view,
    }
}

/// After a settings change: a view switched off goes back to the Inbox, and
/// a topic hidden from the folder pane (Settings, or its own menu) while it
/// is shown goes to all topics, since no Pivot tab or folder node would
/// show where you are any more.
pub fn check_view(app: AppState, ctx: MailCtx, settings: &Settings) {
    let view = *ctx.view.peek();
    match view {
        Some(v) if !view_enabled(settings, v) => go_to_inbox(app, ctx),
        Some(MailView::Topics(Some(t))) if settings.topics_hidden.iter().any(|h| h == t.id()) => show_view(app, ctx, MailView::Topics(None)),
        _ => {}
    }
}

/// The primary account's Inbox (where a switched-off view goes).
fn primary_inbox() -> Option<FolderId> {
    let s = store();
    s.primary_account().ok().flatten().and_then(|a| s.folder_by_role(a.id, FolderRole::Inbox).ok().flatten()).map(|f| f.id)
}

/// Back to the Inbox: the shown folder's account's, else the primary one.
pub fn go_to_inbox(app: AppState, ctx: MailCtx) {
    let s = store();
    let inbox = ctx
        .folder
        .peek()
        .and_then(|f| s.folder(f).ok().flatten())
        .and_then(|f| s.folder_by_role(f.account_id, FolderRole::Inbox).ok().flatten())
        .map(|f| f.id)
        .or_else(primary_inbox);
    if let Some(inbox) = inbox {
        go_to_folder(app, ctx, inbox);
    }
}

/// Show Packages or a topic list (Views in the folder pane, the Topic
/// pivot, the desktop view's switcher). The folder stays as it was, so
/// going back to it restores its list.
pub fn show_view(app: AppState, mut ctx: MailCtx, v: MailView) {
    if !view_enabled(&app.settings.peek(), v) {
        go_to_inbox(app, ctx);
        return;
    }
    close_compose_over_list(app);
    // During a search the list holds the results; the folder's own list
    // was kept when the search started (`search_started`).
    let searching = app.search.peek().active;
    if searching {
        exit_search(app);
    }
    if *ctx.view.peek() == Some(v) {
        // Already there, maybe under an email opened full width (reading
        // pane hidden, or the desktop view): back to the list or cards.
        if *ctx.full_view.peek() {
            ctx.open.set(None);
            ctx.full_view.set(false);
        }
        return;
    }
    if let Some(cur) = *ctx.folder.peek()
        && !searching
        && ctx.person.peek().is_none()
        && ctx.view.peek().is_none()
    {
        let memory = remember(ctx);
        ctx.memory.write().insert(cur, memory);
    }
    // Packages from elsewhere: as left, but no email to come back from.
    ctx.packages.write().restore = false;
    ctx.person.set(None);
    ctx.view.set(Some(v));
    ctx.selection.set(Vec::new());
    ctx.anchor.set(None);
    ctx.focus.set(None);
    ctx.open.set(None);
    ctx.expanded.set(None);
    ctx.select_mode.set(false);
    ctx.full_view.set(false);
}

/// Open Settings on a page (through the shell when there is one).
pub fn open_settings_page(app: AppState, page: crate::settings::PageId) {
    match crate::shell::try_shell() {
        Some(shell) => shell.open_settings(app, page),
        None => {
            let mut dialog = app.settings_dialog;
            dialog.set(Some(page.category().settings_page()));
        }
    }
}

/// Turn the junk filter on or off (Settings and the ribbon).
pub fn set_junk_filter(app: AppState, on: bool) {
    if on {
        let _ = app.check(store().restart_junk_filter());
    }
    app.update_settings(|s| s.junk_filter = on);
    app.bump_mail();
}

/// Turn junk suggestions on or off (Settings and the ribbon).
pub fn set_junk_suggestions(app: AppState, on: bool) {
    app.update_settings(|s| s.junk_suggestions = on);
    app.bump_mail();
}

/// Turn Local AI review on or off (Settings and the ribbon).
pub fn set_junk_shadow(app: AppState, on: bool) {
    if on {
        let _ = app.check(store().start_shadow_review());
    }
    app.update_settings(|s| s.junk_shadow = on);
}

/// The domain of an address ("shop.example" for "a@shop.example").
pub fn domain_of(address: &str) -> String {
    address.rsplit_once('@').map(|(_, d)| d.trim().to_lowercase()).unwrap_or_default()
}

/// The one domain "Everything from {domain}" can cover for these senders:
/// `None` for several domains, none, or a free mail provider.
pub fn offered_domain<'a>(addresses: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut domains: Vec<String> = addresses.into_iter().map(domain_of).collect();
    domains.sort();
    domains.dedup();
    match domains.as_slice() {
        [d] if !d.is_empty() && !topics::is_freemail(d) => Some(d.clone()),
        _ => None,
    }
}

/// The remembered Change topic scope as it applies to these messages: a
/// whole domain only where one is offered (else the sender), so what is
/// shown checked is always what a pick does.
pub fn topic_scope_for(scope: TopicScopeChoice, messages: &[Message]) -> TopicScopeChoice {
    match scope {
        TopicScopeChoice::Domain if offered_domain(messages.iter().map(|m| m.from.address.as_str())).is_none() => TopicScopeChoice::Sender,
        scope => scope,
    }
}

/// Folders whose mail is never sorted into topics, so has none to change.
pub const NO_TOPIC_ROLES: [FolderRole; 5] = [FolderRole::Sent, FolderRole::Drafts, FolderRole::Outbox, FolderRole::Junk, FolderRole::Deleted];

/// Mail that can have a topic: not a draft, and not in a folder of
/// [`NO_TOPIC_ROLES`].
pub fn has_topics(m: &Message, role: Option<FolderRole>) -> bool {
    !m.is_draft && !role.is_some_and(|r| NO_TOPIC_ROLES.contains(&r))
}

/// The messages that can have a topic (see [`has_topics`]), by their own
/// folders: a search or a topic list shows mail from many. A folder under
/// Sent Items, Junk Email or Deleted Items has none either.
pub fn topic_mail(msgs: Vec<Message>) -> Vec<Message> {
    let s = store();
    let mut folders: HashMap<FolderId, bool> = HashMap::new();
    msgs.into_iter().filter(|m| !m.is_draft && *folders.entry(m.folder_id).or_insert_with(|| s.folder_has_topics(m.folder_id).unwrap_or(false))).collect()
}

/// Change topic: for these messages only, or for their senders (or the
/// senders' domain) from now on, in each message's account; `None` goes
/// back to the automatic topic. Mail from your own address only ever
/// changes for the message. One Undo puts every change back. In a single
/// topic's list (`model`), the messages that leave it move the selection
/// on, as a move does.
pub fn change_topic(app: AppState, ctx: MailCtx, model: &ListModel, ids: &[MessageId], topic: Option<topics::Topic>, scope: TopicScopeChoice) {
    let s = store();
    // Sent mail, drafts, junk and deleted mail have no topic.
    let msgs = topic_mail(messages(ids));
    if msgs.is_empty() {
        return;
    }
    let scope = topic_scope_for(scope, &msgs);
    let own: Vec<String> = s.accounts().unwrap_or_default().iter().map(|a| a.email.trim().to_lowercase()).collect();
    let mut targets: Vec<(Option<AccountId>, store::TopicTarget)> = Vec::new();
    for m in &msgs {
        let address = m.from.address.trim().to_lowercase();
        let target = match scope {
            TopicScopeChoice::Message => store::TopicTarget::Message(m.id),
            // Notes to self: never a rule for everything you send yourself.
            _ if own.contains(&address) => store::TopicTarget::Message(m.id),
            // Never a whole free mail provider: the address instead.
            TopicScopeChoice::Domain if !topics::is_freemail(&domain_of(&address)) && !domain_of(&address).is_empty() => {
                store::TopicTarget::Domain(domain_of(&address))
            }
            _ => store::TopicTarget::Sender(address),
        };
        let account = (!matches!(target, store::TopicTarget::Message(_))).then_some(m.account_id);
        if !targets.iter().any(|(a, t)| *a == account && *t == target) {
            targets.push((account, target));
        }
    }
    let cleanup = topic_cleanup(&msgs, &targets);
    // The picks first, then what would still beat them, all at once: one
    // pass over the sorted mail however many senders were picked.
    let wanted = targets.len();
    let all = targets.into_iter().chain(cleanup).enumerate().map(|(i, (account, target))| (account, target, if i < wanted { topic } else { None })).collect();
    let mut changes = match s.set_topics(all) {
        Ok(changes) => changes,
        Err(e) => {
            app.error(format!("Couldn't change the topic: {e}"));
            return;
        }
    };
    if changes.is_empty() {
        return;
    }
    if let Some(MailView::Topics(Some(shown))) = model.view {
        let gone: Vec<MessageId> = msgs.iter().filter(|m| s.topic_for(m.id).ok().flatten().map(|a| a.topic) != Some(shown)).map(|m| m.id).collect();
        select_after_removal(app, ctx, model, &gone);
    }
    app.bump_mail();
    let label = topic.map(|t| t.label()).unwrap_or("the automatic topic");
    let text = match &changes[..changes.len().min(wanted)] {
        [one] => match &one.target {
            store::TopicTarget::Domain(who) | store::TopicTarget::Sender(who) => format!("Mail from {who} will show in {label}"),
            // Back to automatic, but a rule for its sender still decides.
            store::TopicTarget::Message(id) => match topic.is_none().then(|| deciding_rule(*id)).flatten() {
                Some(rule) => format!("This message still follows your rule for {rule}"),
                None => format!("This message will show in {label}"),
            },
        },
        many if many.iter().all(|c| matches!(c.target, store::TopicTarget::Message(_))) => format!("Updated {}", items(many.len())),
        many => format!("Updated {} senders", many.len()),
    };
    let undo = if changes.len() == 1 { UndoAction::Topic(changes.remove(0)) } else { UndoAction::Topics(changes) };
    app.toast_with_undo(text, undo);
}

/// What would still beat a sender or domain choice for the messages it was
/// picked on: a topic set for one of them alone, and for a domain, a rule
/// for one of their addresses (an address rule beats a domain rule). They
/// are removed with the change, so the messages follow it, and come back
/// with its Undo.
fn topic_cleanup(msgs: &[Message], targets: &[(Option<AccountId>, store::TopicTarget)]) -> Vec<(Option<AccountId>, store::TopicTarget)> {
    let s = store();
    let mut out = Vec::new();
    let covered: Vec<&Message> = msgs
        .iter()
        .filter(|m| {
            let address = m.from.address.trim().to_lowercase();
            targets.iter().any(|(a, t)| {
                *a == Some(m.account_id)
                    && match t {
                        store::TopicTarget::Sender(x) => *x == address,
                        store::TopicTarget::Domain(d) => *d == domain_of(&address),
                        store::TopicTarget::Message(_) => false,
                    }
            })
        })
        .collect();
    if covered.is_empty() {
        return out;
    }
    let ids: Vec<MessageId> = covered.iter().map(|m| m.id).collect();
    let overridden = s.topic_overrides(&ids).unwrap_or_default();
    let rules = s.topic_rules().unwrap_or_default();
    for m in covered {
        if overridden.contains_key(&m.id) {
            out.push((None, store::TopicTarget::Message(m.id)));
        }
        let address = m.from.address.trim().to_lowercase();
        let by_domain = targets.iter().any(|(a, t)| *a == Some(m.account_id) && *t == store::TopicTarget::Domain(domain_of(&address)));
        if !by_domain {
            continue;
        }
        for r in rules.iter().map(|r| &r.rule).filter(|r| !r.is_domain && r.pattern == address && (r.account_key == 0 || r.account_key == m.account_id)) {
            let t = (Some(r.account_key).filter(|k| *k != 0), store::TopicTarget::Sender(address.clone()));
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

/// The sender or domain rule that sorts a message, if one does.
fn deciding_rule(id: MessageId) -> Option<String> {
    let a = store().topic_for(id).ok().flatten()?;
    a.reasons.into_iter().find_map(|r| match r {
        topics::Reason::SenderRule { pattern } | topics::Reason::DomainRule { pattern } => Some(pattern),
        _ => None,
    })
}

/// A command from an order card's More menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderMenuCmd {
    MarkDelivered,
    MarkNotDelivered,
    Hide,
    IgnoreSender,
    WhyHere,
}

impl OrderMenuCmd {
    pub fn id(self) -> &'static str {
        match self {
            OrderMenuCmd::MarkDelivered => "o_delivered",
            OrderMenuCmd::MarkNotDelivered => "o_not_delivered",
            OrderMenuCmd::Hide => "o_hide",
            OrderMenuCmd::IgnoreSender => "o_ignore",
            OrderMenuCmd::WhyHere => "o_why",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [OrderMenuCmd::MarkDelivered, OrderMenuCmd::MarkNotDelivered, OrderMenuCmd::Hide, OrderMenuCmd::IgnoreSender, OrderMenuCmd::WhyHere]
            .into_iter()
            .find(|c| c.id() == s)
    }
}

/// The status override that makes `order` delivered (or not): none, back
/// to what the mail says, when the mail already says so; else the user's
/// own. Decided by what the mail says, not by whether there is an
/// override: "not delivered" over mail that says Delivered must stay an
/// override, or the order falls back to Delivered.
pub fn delivered_override(order: &orders::Order, delivered: bool) -> Option<orders::OrderStatus> {
    use orders::OrderStatus::{Delivered, InTransit};
    let mail_delivered = order.mail_status.map(|s| s == Delivered);
    match (order.overridden, mail_delivered) {
        (true, Some(d)) if d == delivered => None,
        _ if delivered => Some(Delivered),
        _ => Some(InTransit),
    }
}

/// What "Don't track orders from…" stops tracking for an order: the shop's
/// domain, or, for a seller on a free mail domain or a shop on a
/// marketplace relay (whose domain is many sellers'), the sender's address
/// (`sender_of` an email of the order).
pub fn ignore_target(order: &orders::Order, sender_of: impl Fn(MessageId) -> Option<String>) -> Option<String> {
    let m = &order.merchant;
    let domain = m.domain.trim().to_lowercase();
    if !domain.is_empty() && m.key == m.domain && !topics::is_freemail(&domain) {
        return Some(domain);
    }
    if m.key.contains('@') && !m.key.starts_with("name:") {
        return Some(m.key.clone());
    }
    let newest = order.events.iter().max_by_key(|e| e.at)?;
    sender_of(newest.message_id).map(|a| a.trim().to_lowercase()).filter(|a| a.contains('@'))
}

/// The sender's address of a message.
pub fn order_sender(id: MessageId) -> Option<String> {
    store().message(id).ok().flatten().map(|m| m.from.address)
}

/// Run an order card's More menu command on the order with this key.
/// "Why is this here?" is shown by the card itself.
pub fn order_action(app: AppState, key: &str, cmd: OrderMenuCmd) {
    let s = store();
    let Some(order) = app.check(s.order_by_key(key)).flatten() else {
        app.error("That order is no longer in Packages.");
        return;
    };
    let name = if order.merchant.name.trim().is_empty() { order.merchant.domain.clone() } else { order.merchant.name.clone() };
    let state = |change: store::OrderStateChange, text: String| {
        if let Some(previous) = app.check(s.set_order_state(key, change)) {
            app.bump_mail();
            app.toast_with_undo(text, UndoAction::OrderState { key: key.to_string(), previous });
        }
    };
    match cmd {
        OrderMenuCmd::MarkDelivered => state(store::OrderStateChange::Override(delivered_override(&order, true)), "Marked as delivered".into()),
        OrderMenuCmd::MarkNotDelivered => state(store::OrderStateChange::Override(delivered_override(&order, false)), "Marked as not delivered".into()),
        OrderMenuCmd::Hide => state(store::OrderStateChange::Hide(true), format!("Removed the order from {name} from Packages")),
        OrderMenuCmd::IgnoreSender => {
            let Some(pattern) = ignore_target(&order, order_sender) else { return };
            if app.check(s.ignore_order_sender(&pattern)).is_some() {
                app.bump_mail();
                app.toast_with_undo(format!("Orders from {pattern} won't be tracked"), UndoAction::OrderIgnore(pattern));
            }
        }
        OrderMenuCmd::WhyHere => {}
    }
}

/// A search starts: remember the folder's list (not a view's, which has
/// none to keep, nor a person's, which isn't the folder's) and the view to
/// come back to; start the results empty. A
/// view has no folder on screen, so its search covers all folders (and the
/// scope chosen for folders comes back when it ends).
pub fn search_started(mut ctx: MailCtx) {
    let view = *ctx.view.peek();
    ctx.search_return.set(view);
    let scope = *ctx.search_scope.peek();
    if view.is_some() && scope != SearchScope::AllFolders {
        ctx.scope_return.set(Some(scope));
        ctx.search_scope.set(SearchScope::AllFolders);
    }
    if let Some(f) = *ctx.folder.peek()
        && view.is_none()
        && ctx.person.peek().is_none()
    {
        let memory = remember(ctx);
        ctx.memory.write().insert(f, memory);
    }
    ctx.clear_selection();
    ctx.focus.set(None);
    ctx.expanded.set(None);
    ctx.full_view.set(false);
}

/// The search ended: back to the view it started from (unless the user
/// went somewhere else meanwhile), else the folder's remembered list (not
/// into a view or a person's mail, which have their own).
pub fn search_ended(mut ctx: MailCtx) {
    let back = ctx.search_return.peek().filter(|v| *ctx.view.peek() == Some(*v));
    ctx.search_return.set(None);
    let saved = *ctx.scope_return.peek();
    if let Some(scope) = saved {
        ctx.scope_return.set(None);
        ctx.search_scope.set(scope);
    }
    if back.is_some() {
        ctx.clear_selection();
        ctx.focus.set(None);
        return;
    }
    if ctx.view.peek().is_some() || ctx.person.peek().is_some() {
        return;
    }
    if let Some(m) = ctx.folder.peek().and_then(|f| ctx.memory.peek().get(&f).cloned()) {
        ctx.selection.set(m.selection);
        ctx.anchor.set(m.anchor);
        ctx.focus.set(m.focus);
        ctx.open.set(m.open);
    }
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
        "not_junk" => not_junk(app, ctx, model, ids),
        "restore" => restore(app, ctx, model, ids),
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
            // An allow rule for this account only. It doesn't load pictures
            // or quiet phishing warnings.
            let mut rules = Vec::new();
            for m in messages(&ids) {
                if let Some(change) = app.check(s.set_sender_rule(Some(m.account_id), &m.from.address, &m.from.name, junk::RuleAction::Allow)) {
                    rules.push(change);
                }
            }
            if !rules.is_empty() {
                app.toast_with_undo("Never blocked in this account", UndoAction::Junk { events: Vec::new(), rules, moves: Vec::new() });
                app.bump_mail();
            }
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
        "topic_reset" => change_topic(app, ctx, model, &ids, None, *ctx.topic_scope.peek()),
        "topic_dialog" => {
            if !ids.is_empty() {
                ctx.dialog.set(Some(MailDialog::ChangeTopic { ids }));
            }
        }
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
            } else if let Some(t) = other.strip_prefix("topic:").and_then(topics::Topic::parse) {
                change_topic(app, ctx, model, &ids, Some(t), *ctx.topic_scope.peek());
            } else {
                return false;
            }
        }
    }
    true
}

/// The folder `ctx.folder` names is gone (deleted for good, its account
/// removed): show `inbox` instead. While Packages, a topic list or a
/// person's mail is on screen, only the folder to come back to changes.
pub fn folder_gone(app: AppState, mut ctx: MailCtx, inbox: FolderId) {
    if ctx.view.peek().is_some() || ctx.person.peek().is_some() {
        ctx.folder.set(Some(inbox));
    } else {
        go_to_folder(app, ctx, inbox);
    }
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use super::*;

    thread_local! {
        static ROOT: Cell<Option<(AppState, MailCtx)>> = const { Cell::new(None) };
    }

    fn root() -> Element {
        let app = crate::state::provide_app_state();
        let ctx = use_context_provider(MailCtx::new);
        ROOT.with(|r| r.set(Some((app, ctx))));
        rsx! {}
    }

    /// Run `f` with the app state and a fresh Mail context (default settings).
    fn with_mail(f: impl FnOnce(AppState, MailCtx)) {
        crate::state::init(Arc::new(axentra_core::Store::open_in_memory().unwrap()), None);
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        let (app, ctx) = ROOT.with(|r| r.get()).expect("the mail context");
        dom.in_runtime(|| {
            let mut settings = app.settings;
            settings.set(Settings::default());
            f(app, ctx)
        });
    }

    #[test]
    fn a_folder_click_leaves_a_view_even_for_the_same_folder() {
        with_mail(|app, mut ctx| {
            go_to_folder(app, ctx, 900_001);
            ctx.tab.set(InferenceClass::Other);
            show_view(app, ctx, MailView::Topics(None));
            assert_eq!(*ctx.view.peek(), Some(MailView::Topics(None)));
            assert_eq!(*ctx.folder.peek(), Some(900_001), "the folder is kept for coming back");
            // The early return for "same folder" must not keep the view.
            go_to_folder(app, ctx, 900_001);
            assert_eq!(*ctx.view.peek(), None);
            assert_eq!(*ctx.tab.peek(), InferenceClass::Other, "the folder's Focused/Other tab comes back");
            // A person's mail and a view replace each other.
            show_view(app, ctx, MailView::Packages);
            show_person(app, ctx, PersonView { contact: 3, name: "Ana".into(), address: "ana@x.example".into() });
            assert_eq!(*ctx.view.peek(), None);
            show_view(app, ctx, MailView::Topics(Some(topics::Topic::Travel)));
            assert!(ctx.person.peek().is_none());
            assert_eq!(*ctx.tab.peek(), InferenceClass::Other);
            go_to_folder(app, ctx, 900_001);
            assert_eq!(*ctx.tab.peek(), InferenceClass::Other, "still the folder's own tab after two views");
        });
    }

    #[test]
    fn the_folder_behind_a_view_going_away_keeps_the_view() {
        with_mail(|app, ctx| {
            go_to_folder(app, ctx, 900_031);
            show_view(app, ctx, MailView::Topics(Some(topics::Topic::Travel)));
            folder_gone(app, ctx, 900_032);
            assert_eq!(*ctx.view.peek(), Some(MailView::Topics(Some(topics::Topic::Travel))), "the view stays on screen");
            assert_eq!(*ctx.folder.peek(), Some(900_032), "the Inbox to come back to");
            show_person(app, ctx, PersonView { contact: 3, name: "Ana".into(), address: "ana@x.example".into() });
            folder_gone(app, ctx, 900_033);
            assert!(ctx.person.peek().is_some());
            // The folder itself on screen: the Inbox replaces it.
            go_to_folder(app, ctx, 900_031);
            folder_gone(app, ctx, 900_032);
            assert_eq!((*ctx.folder.peek(), *ctx.view.peek()), (Some(900_032), None));
        });
    }

    #[test]
    fn dont_track_never_covers_a_free_mail_domain_or_a_relay() {
        let order = |domain: &str, key: &str| orders::Order {
            merchant: orders::Merchant { domain: domain.into(), name: "Shop".into(), key: key.into() },
            events: vec![orders::OrderEvent { message_id: 7, at: Utc::now(), status: None, subject: String::new() }],
            ..Default::default()
        };
        let sender = |id: MessageId| (id == 7).then(|| "Hello@Shops.Example".to_string());
        assert_eq!(ignore_target(&order("brightleaf.example", "brightleaf.example"), sender).as_deref(), Some("brightleaf.example"));
        assert_eq!(ignore_target(&order("gmail.com", "janespottery@gmail.com"), sender).as_deref(), Some("janespottery@gmail.com"));
        assert_eq!(ignore_target(&order("shops.example", "name:clay & co"), sender).as_deref(), Some("hello@shops.example"));
        assert_eq!(ignore_target(&order("shops.example", "name:clay & co"), |_| None), None);
    }

    #[test]
    fn junk_and_deleted_mail_has_no_topic() {
        let m = Message::default();
        assert!(has_topics(&m, Some(FolderRole::Inbox)) && has_topics(&m, None));
        for role in [FolderRole::Junk, FolderRole::Deleted, FolderRole::Sent] {
            assert!(!has_topics(&m, Some(role)), "{role:?}");
        }
    }

    #[test]
    fn a_search_from_a_view_returns_to_it() {
        with_mail(|app, mut ctx| {
            go_to_folder(app, ctx, 900_011);
            ctx.tab.set(InferenceClass::Other);
            show_view(app, ctx, MailView::Packages);
            ctx.selection.set(vec![RowKey::Msg(5)]);
            // Narrowed to the folder before: a view has none on screen.
            ctx.search_scope.set(SearchScope::CurrentFolder);
            search_started(ctx);
            assert_eq!(*ctx.search_scope.peek(), SearchScope::AllFolders);
            assert_eq!(*ctx.search_return.peek(), Some(MailView::Packages));
            assert!(ctx.selection.peek().is_empty());
            assert_eq!(ctx.memory.peek().get(&900_011).map(|m| m.tab), Some(InferenceClass::Other), "the folder's memory is untouched");
            search_ended(ctx);
            assert_eq!(*ctx.view.peek(), Some(MailView::Packages));
            assert_eq!(*ctx.search_scope.peek(), SearchScope::CurrentFolder, "the folders' scope is back");
            // Going to a folder during the search: the search ends there.
            search_started(ctx);
            go_to_folder(app, ctx, 900_012);
            search_ended(ctx);
            assert_eq!(*ctx.view.peek(), None);
            assert_eq!(*ctx.folder.peek(), Some(900_012));
            assert_eq!(*ctx.search_scope.peek(), SearchScope::CurrentFolder, "the next folder search is narrowed again");
            // A search from a folder leaves the scope alone.
            search_started(ctx);
            assert_eq!(*ctx.search_scope.peek(), SearchScope::CurrentFolder);
            search_ended(ctx);
            assert_eq!(*ctx.search_scope.peek(), SearchScope::CurrentFolder);
        });
    }

    #[test]
    fn a_search_and_a_persons_mail_leave_the_folders_list_alone() {
        with_mail(|app, mut ctx| {
            let ana = || PersonView { contact: 3, name: "Ana".into(), address: "ana@x.example".into() };
            let open = |id: MessageId| Some(Open { message: id, conversation: None, mark: Vec::new() });
            // Email A open in the Inbox.
            go_to_folder(app, ctx, 900_041);
            ctx.selection.set(vec![RowKey::Msg(1)]);
            ctx.open.set(open(1));
            ctx.list_scroll.set(400.0);
            // Ana's email B, then a search from her mail.
            show_person(app, ctx, ana());
            ctx.selection.set(vec![RowKey::Msg(2)]);
            ctx.open.set(open(2));
            ctx.list_scroll.set(90.0);
            search_started(ctx);
            let m = ctx.memory.peek().get(&900_041).cloned().unwrap();
            assert_eq!((m.selection, m.open.map(|o| o.message), m.scroll), (vec![RowKey::Msg(1)], Some(1), 400.0), "the Inbox's own list");
            search_ended(ctx);
            assert!(ctx.selection.peek().is_empty() && ctx.open.peek().is_none(), "the Inbox's list is not put into Ana's");
            assert_eq!(ctx.memory.peek().get(&900_041).and_then(|m| m.open.as_ref().map(|o| o.message)), Some(1));
            go_to_folder(app, ctx, 900_041);
            // A search from the Inbox, left for Ana's mail: her list starts empty.
            search_started(ctx);
            ctx.selection.set(vec![RowKey::Msg(3)]);
            show_person(app, ctx, ana());
            search_ended(ctx);
            assert!(ctx.person.peek().is_some());
            assert!(ctx.selection.peek().is_empty() && ctx.open.peek().is_none() && ctx.focus.peek().is_none(), "nothing of the Inbox's in Ana's list");
        });
    }

    #[test]
    fn leaving_a_search_keeps_the_folders_own_list() {
        with_mail(|app, mut ctx| {
            let mut search = app.search;
            for leave in 0..3 {
                go_to_folder(app, ctx, 900_021);
                ctx.selection.set(vec![RowKey::Msg(1)]);
                ctx.list_scroll.set(400.0);
                search_started(ctx);
                search.set(SearchState { query: "invoice".into(), active: true, serial: 1 });
                // A result opened in the search.
                ctx.selection.set(vec![RowKey::Msg(2)]);
                ctx.open.set(Some(Open { message: 2, conversation: None, mark: Vec::new() }));
                ctx.list_scroll.set(10.0);
                match leave {
                    0 => show_view(app, ctx, MailView::Packages),
                    1 => show_person(app, ctx, PersonView { contact: 3, name: "Ana".into(), address: "ana@x.example".into() }),
                    _ => go_to_folder(app, ctx, 900_022),
                }
                assert!(!search.peek().active);
                let m = ctx.memory.peek().get(&900_021).cloned().unwrap();
                assert_eq!((m.selection, m.open, m.scroll), (vec![RowKey::Msg(1)], None, 400.0), "case {leave}");
                search.set(SearchState::default());
            }
            // Back from Packages (which replaces the list): the list, shown
            // anew, scrolls to where the folder was left.
            go_to_folder(app, ctx, 900_021);
            ctx.list_scroll.set(640.0);
            show_view(app, ctx, MailView::Packages);
            ctx.list_scroll.set(0.0);
            go_to_folder(app, ctx, 900_021);
            assert_eq!(*ctx.list_scroll.peek(), 640.0);
        });
    }

    #[test]
    fn a_message_that_leaves_a_topic_list_moves_the_selection_on() {
        use topics::Topic::{Finance, Newsletters};
        with_mail(|app, mut ctx| {
            let s = store();
            let mut acct = Account { kind: AccountKind::Local, email: "me@home.example".into(), ..Default::default() };
            let account = s.add_account(&mut acct).unwrap();
            let inbox = s.ensure_role_folder(account, FolderRole::Inbox).unwrap();
            let add = |from: &str| {
                let mut m = Message { account_id: account, folder_id: inbox, subject: "Hello".into(), from: EmailAddress::new("", from), received_at: Utc::now(), ..Default::default() };
                let id = s.insert_message(&mut m, None).unwrap();
                s.set_topic(None, axentra_core::store::TopicTarget::Message(id), Some(Newsletters)).unwrap();
                id
            };
            let (a, b) = (add("a@news.example"), add("b@news.example"));
            // Newsletters, with `a` selected and open.
            let mut model = ListModel::from_keys(vec![RowKey::Msg(a), RowKey::Msg(b)]);
            for id in [a, b] {
                model.ids.insert(RowKey::Msg(id), vec![id]);
                model.opens.insert(RowKey::Msg(id), Open { message: id, conversation: None, mark: Vec::new() });
            }
            model.view = Some(MailView::Topics(Some(Newsletters)));
            ctx.selection.set(vec![RowKey::Msg(a)]);
            ctx.focus.set(Some(RowKey::Msg(a)));
            ctx.open.set(model.opens.get(&RowKey::Msg(a)).cloned());
            change_topic(app, ctx, &model, &[a], Some(Finance), TopicScopeChoice::Message);
            assert_eq!(*ctx.selection.peek(), vec![RowKey::Msg(b)], "on to the next item");
            assert_eq!(ctx.open.peek().as_ref().map(|o| o.message), Some(b));
            // Changed to the topic shown: it stays, and so does the selection.
            change_topic(app, ctx, &model, &[b], Some(Newsletters), TopicScopeChoice::Message);
            assert_eq!(*ctx.selection.peek(), vec![RowKey::Msg(b)]);
            // All topics: nothing leaves the list.
            model.view = Some(MailView::Topics(None));
            change_topic(app, ctx, &model, &[b], Some(Finance), TopicScopeChoice::Message);
            assert_eq!(*ctx.selection.peek(), vec![RowKey::Msg(b)]);
        });
    }

    #[test]
    fn a_sender_or_domain_change_moves_the_message_it_was_picked_on() {
        use topics::Topic::{Finance, Receipts, Travel};
        with_mail(|app, ctx| {
            let s = store();
            let mut acct = Account { kind: AccountKind::Local, email: "me@home.example".into(), ..Default::default() };
            let account = s.add_account(&mut acct).unwrap();
            let inbox = s.ensure_role_folder(account, FolderRole::Inbox).unwrap();
            let sent = s.ensure_role_folder(account, FolderRole::Sent).unwrap();
            let add = |folder, from: &str| {
                let mut m = Message {
                    account_id: account,
                    folder_id: folder,
                    subject: "Hello".into(),
                    from: EmailAddress::new("", from),
                    received_at: Utc::now(),
                    ..Default::default()
                };
                s.insert_message(&mut m, None).unwrap()
            };
            let topic = |id| s.topic_for(id).unwrap().map(|a| a.topic);
            let last = || app.toasts.peek().last().cloned().unwrap();
            let a = add(inbox, "x@shop.example");
            change_topic(app, ctx, &ListModel::default(), &[a], Some(Travel), TopicScopeChoice::Message);
            assert_eq!(topic(a), Some(Travel));
            // "From now on" for the sender: this message too, though it had
            // a topic of its own.
            change_topic(app, ctx, &ListModel::default(), &[a], Some(Receipts), TopicScopeChoice::Sender);
            assert_eq!(topic(a), Some(Receipts));
            assert_eq!(last().text, format!("Mail from x@shop.example will show in {}", Receipts.label()));
            // The whole domain, over the address rule.
            change_topic(app, ctx, &ListModel::default(), &[a], Some(Finance), TopicScopeChoice::Domain);
            assert_eq!(topic(a), Some(Finance));
            assert_eq!(last().text, format!("Mail from shop.example will show in {}", Finance.label()));
            // One Undo puts the address rule back.
            app.run_undo(&last().undo.unwrap());
            assert_eq!(topic(a), Some(Receipts));
            // Back to automatic for the message alone: the rule still decides,
            // and the toast says so.
            change_topic(app, ctx, &ListModel::default(), &[a], None, TopicScopeChoice::Message);
            assert_eq!(last().text, "This message still follows your rule for x@shop.example");

            // Your own mail: no rule for your address, ever.
            let mine = add(sent, "me@home.example");
            let note = add(inbox, "Me@Home.example");
            let toasts = app.toasts.peek().len();
            change_topic(app, ctx, &ListModel::default(), &[mine], Some(Travel), TopicScopeChoice::Sender);
            assert_eq!(app.toasts.peek().len(), toasts, "Sent Items mail has no topic to change");
            change_topic(app, ctx, &ListModel::default(), &[note], Some(Travel), TopicScopeChoice::Sender);
            assert_eq!(topic(note), Some(Travel));
            assert_eq!(last().text, format!("This message will show in {}", Travel.label()));
            assert!(s.topic_rules().unwrap().iter().all(|r| r.rule.pattern != "me@home.example"));
            // Nor is it offered there.
            let offered = |id, role, folder| {
                let t = super::super::menus::Target::for_ids(&[id], Some(role), Some(folder), false, false);
                super::super::menus::message_menu(&t, Some(folder)).iter().any(|i| i.id == "topic_menu")
            };
            assert!(!offered(mine, FolderRole::Sent, sent));
            assert!(offered(note, FolderRole::Inbox, inbox));
        });
    }

    #[test]
    fn a_switched_off_view_is_not_shown() {
        with_mail(|app, ctx| {
            let mut settings = app.settings;
            settings.set(Settings { topics_view: false, ..Settings::default() });
            show_view(app, ctx, MailView::Topics(None));
            assert_eq!(*ctx.view.peek(), None);
            assert!(view_enabled(&app.settings.peek(), MailView::Packages));
            assert!(!view_enabled(&app.settings.peek(), MailView::Topics(Some(topics::Topic::Travel))));
        });
    }

    #[test]
    fn domains_and_order_commands() {
        assert_eq!(domain_of("Ana@Shop.Example"), "shop.example");
        assert_eq!(domain_of("no-at-sign"), "");
        for cmd in [OrderMenuCmd::MarkDelivered, OrderMenuCmd::MarkNotDelivered, OrderMenuCmd::Hide, OrderMenuCmd::IgnoreSender, OrderMenuCmd::WhyHere] {
            assert_eq!(OrderMenuCmd::parse(cmd.id()), Some(cmd));
        }
        assert_eq!(TopicScopeChoice::parse("domain"), Some(TopicScopeChoice::Domain));
        assert_eq!(TopicScopeChoice::default(), TopicScopeChoice::Sender);
    }

    #[test]
    fn a_remembered_domain_scope_applies_only_where_offered() {
        let from = |address: &str| Message { from: EmailAddress::new("", address), ..Default::default() };
        let one = [from("a@shop.example"), from("B@Shop.Example")];
        let two = [from("b@foo.example"), from("c@bar.example")];
        let free = [from("someone@gmail.com")];
        assert_eq!(offered_domain(one.iter().map(|m| m.from.address.as_str())).as_deref(), Some("shop.example"));
        assert_eq!(topic_scope_for(TopicScopeChoice::Domain, &one), TopicScopeChoice::Domain);
        assert_eq!(topic_scope_for(TopicScopeChoice::Domain, &two), TopicScopeChoice::Sender, "two domains");
        assert_eq!(topic_scope_for(TopicScopeChoice::Domain, &free), TopicScopeChoice::Sender, "free mail");
        assert_eq!(topic_scope_for(TopicScopeChoice::Message, &two), TopicScopeChoice::Message);
        // The menu says what a pick applies to, and offers no scope picks of its own.
        let items = super::super::menus::change_topic_items(&two, None, TopicScopeChoice::Domain);
        assert!(items.iter().all(|i| !i.radio && !i.id.starts_with("topic_scope:")));
        assert!(items.iter().any(|i| i.header && i.label == "For these senders from now on"), "{:?}", items.iter().map(|i| &i.label).collect::<Vec<_>>());
        let items = super::super::menus::change_topic_items(&one[..1], None, TopicScopeChoice::Domain);
        assert!(items.iter().any(|i| i.header && i.label == "For everything from shop.example"));
    }

    #[test]
    fn hiding_the_topic_shown_goes_to_all_topics() {
        with_mail(|app, ctx| {
            show_view(app, ctx, MailView::Topics(Some(topics::Topic::Travel)));
            let mut settings = app.settings;
            settings.write().topics_hidden.push("finance".into());
            check_view(app, ctx, &app.settings.peek().clone());
            assert_eq!(*ctx.view.peek(), Some(MailView::Topics(Some(topics::Topic::Travel))), "another topic hidden");
            settings.write().topics_hidden.push("travel".into());
            check_view(app, ctx, &app.settings.peek().clone());
            assert_eq!(*ctx.view.peek(), Some(MailView::Topics(None)));
        });
    }

    #[test]
    fn marking_delivered_or_not_goes_by_what_the_mail_says() {
        use orders::OrderStatus::{Delivered, InTransit, Shipped};
        let order = |status, overridden, mail| orders::Order { status, overridden, mail_status: Some(mail), ..Default::default() };
        // The mail says Delivered: "not delivered" is an override, and stays
        // one after the user went back and forth.
        assert_eq!(delivered_override(&order(Delivered, false, Delivered), false), Some(InTransit));
        assert_eq!(delivered_override(&order(InTransit, true, Delivered), true), None, "back to the mail");
        assert_eq!(delivered_override(&order(Delivered, true, Delivered), false), Some(InTransit));
        // The mail says Shipped: "delivered" is the user's, "not" undoes it.
        assert_eq!(delivered_override(&order(Shipped, false, Shipped), true), Some(Delivered));
        assert_eq!(delivered_override(&order(Delivered, true, Shipped), false), None);
        // An order built before the mail's status was kept.
        let old = orders::Order { status: Delivered, overridden: true, mail_status: None, ..Default::default() };
        assert_eq!(delivered_override(&old, false), Some(InTransit));
    }

    #[test]
    fn the_spam_menu_flips_each_switch_and_back() {
        with_mail(|app, ctx| {
            let model = ListModel::default();
            let get = |id: &str| {
                let s = app.settings.peek();
                match id {
                    "spam_filter" => s.junk_filter,
                    "spam_suggestions" => s.junk_suggestions,
                    _ => s.junk_shadow,
                }
            };
            for id in ["spam_filter", "spam_suggestions", "spam_ai"] {
                let was = get(id);
                crate::mail::ribbon::on_command(app, ctx, &model, id);
                assert_eq!(get(id), !was, "{id}");
                crate::mail::ribbon::on_command(app, ctx, &model, id);
                assert_eq!(get(id), was, "{id}");
            }
        });
    }

    #[test]
    fn choosing_the_folder_already_shown_closes_a_full_width_email() {
        with_mail(|app, mut ctx| {
            go_to_folder(app, ctx, 900_001);
            ctx.open.set(Some(Open { message: 7, conversation: None, mark: Vec::new() }));
            ctx.full_view.set(true);
            go_to_folder(app, ctx, 900_001);
            assert!(!*ctx.full_view.peek() && ctx.open.peek().is_none(), "the list is back");
            // Beside the list, an open email stays open.
            ctx.open.set(Some(Open { message: 7, conversation: None, mark: Vec::new() }));
            go_to_folder(app, ctx, 900_001);
            assert!(ctx.open.peek().is_some());
        });
    }

    #[test]
    fn choosing_a_list_closes_a_message_being_written_only_when_it_covers_the_list() {
        with_mail(|app, ctx| {
            go_to_folder(app, ctx, 900_001);
            app.compose(ComposeRequest::new_message());
            go_to_folder(app, ctx, 900_001);
            assert!(app.compose.peek().is_some(), "beside the list it stays");
            app.update_settings(|s| s.reading_pane = ReadingPanePosition::Hidden);
            app.compose(ComposeRequest { popout: true, ..ComposeRequest::new_message() });
            go_to_folder(app, ctx, 900_001);
            assert!(app.compose.peek().is_some(), "its own window stays");
            app.compose(ComposeRequest::new_message());
            go_to_folder(app, ctx, 900_001);
            assert!(app.compose.peek().is_none(), "the list is back");
        });
    }

    #[test]
    fn showing_the_view_already_shown_closes_a_full_width_email() {
        with_mail(|app, mut ctx| {
            show_view(app, ctx, MailView::Packages);
            ctx.open.set(Some(Open { message: 7, conversation: None, mark: Vec::new() }));
            ctx.full_view.set(true);
            show_view(app, ctx, MailView::Packages);
            assert!(!*ctx.full_view.peek() && ctx.open.peek().is_none(), "the cards are back");
            // Beside the list, an open email stays open.
            ctx.open.set(Some(Open { message: 7, conversation: None, mark: Vec::new() }));
            show_view(app, ctx, MailView::Packages);
            assert!(ctx.open.peek().is_some());
        });
    }
}
