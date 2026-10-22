//! Messages that wait before going out: the "Undo send" delay and
//! scheduled sends. Both outlive the compose form, so they run as
//! app-lifetime tasks (`spawn_forever`).

use std::collections::HashMap;
use std::time::Duration as StdDuration;

use axentra_core::{MessageId, MoveRecord};
use chrono::{DateTime, Utc};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;

use crate::state::{AppState, ComposeRequest, UndoAction, store, sync};

/// Held sends this session is watching (the background pass skips them),
/// with their "Sending…" bar.
static WATCHED: GlobalSignal<HashMap<MessageId, u64>> = Signal::global(HashMap::new);
/// Whether the background pass runs in this app instance.
static BACKGROUND: GlobalSignal<bool> = Signal::global(|| false);

/// A scheduled send this overdue was missed (the pass runs every 15 s).
const LATE: chrono::TimeDelta = chrono::TimeDelta::minutes(2);

/// Ask the sync engine to deliver what is waiting in Outbox.
pub fn flush_outbox() {
    if let Some(s) = sync() {
        s.send(axentra_sync::Command::FlushOutbox);
    }
}

/// Show "Sending…" with Undo and a countdown for a held message and send
/// it when the delay ends. Undo (the toast's button, or Ctrl+Z anywhere
/// outside a text field: the send is on the app's undo stack while it is
/// held) moves the message back to Drafts and reopens it.
pub fn watch(app: AppState, record: MoveRecord, until: DateTime<Utc>, subject: String) {
    let id = record.message_id;
    let toast = app.toast_undo_send("Sending…", UndoAction::Moves(vec![record.clone()]), until);
    WATCHED.write().insert(id, toast);
    spawn_forever(async move {
        loop {
            tokio::time::sleep(StdDuration::from_millis(200)).await;
            let s = store();
            let msg = s.message(id).ok().flatten();
            let back_in_drafts = msg.as_ref().is_some_and(|m| m.is_draft && m.folder_id == record.from_folder);
            let still_held = msg.as_ref().is_some_and(|m| m.is_draft && m.folder_id != record.from_folder);
            if back_in_drafts {
                // Undo: reopen the draft exactly as it was.
                app.forget_undo(toast);
                let _ = s.unqueue_send(id);
                app.dismiss_toast(toast);
                app.bump_mail();
                app.compose(ComposeRequest { draft: Some(id), ..ComposeRequest::new_message() });
                break;
            }
            if !still_held {
                // Moved or deleted elsewhere: nothing left to send.
                app.forget_undo(toast);
                app.dismiss_toast(toast);
                break;
            }
            if Utc::now() >= until {
                // Off the undo stack before it goes: Ctrl+Z must not pull
                // back a message that was sent.
                app.forget_undo(toast);
                app.dismiss_toast(toast);
                match s.release_queued_send(id) {
                    Ok(true) => {
                        flush_outbox();
                        app.bump_mail();
                        app.toast("Sent");
                    }
                    Ok(false) => {}
                    Err(e) => {
                        tracing::warn!("release held send {id}: {e}");
                        app.error(format!("Couldn't send \"{subject}\". It's in Outbox."));
                    }
                }
                break;
            }
        }
        WATCHED.write().remove(&id);
    });
}

/// Ctrl+Z outside a text field (Mail's keys and the app's), and in a
/// compose form right after its Send (`from_form`, see [`SEND_UNDO_JS`]):
/// a send still starting is taken back before it goes; otherwise the most
/// recent action is undone, a held send among them. From a form only a
/// held send is: when the app refused that send (no subject, say), an
/// older action is left alone.
pub fn undo(app: AppState, from_form: bool) {
    if super::actions::take_back_starting_send() {
        return;
    }
    if !from_form || app.newest_undo_bar().is_some_and(|bar| WATCHED.peek().values().any(|&t| t == bar)) {
        app.undo_last();
    }
}

/// Ctrl+Z in a compose form after the app took its Ctrl+Enter or Alt+S:
/// until the form goes (or the app refuses the send and calls
/// `__axSendRefused`), it would undo typing in the message about to go
/// out, so the page sends the form's id here ([`undo`]) instead. Ctrl+Y
/// there does nothing meanwhile.
const SEND_UNDO_JS: &str = r#"
window.__axSendUndoTo = (id) => dioxus.send(id);
if (!window.__axSendUndo) {
  const sending = (window.__axSendUndo = new WeakSet());
  const form = (t) => (t && t.closest ? t.closest(".ax-compose") : null);
  const ctrl = (e) => e.ctrlKey || e.metaKey;
  document.addEventListener("keydown", (e) => {
    const f = form(e.target), k = (e.key || "").toLowerCase();
    if (f && e.defaultPrevented && !e.isComposing && !e.shiftKey && (ctrl(e) ? !e.altKey && e.key === "Enter" : e.altKey && k === "s")) sending.add(f);
  });
  window.addEventListener("keydown", (e) => {
    const f = form(e.target), k = (e.key || "").toLowerCase();
    if (!f || !sending.has(f) || !ctrl(e) || e.altKey || (k !== "z" && k !== "y")) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    if (k === "z" && !e.shiftKey && !e.repeat) window.__axSendUndoTo(f.id);
  }, true);
  window.__axSendRefused = (id) => { const f = document.getElementById(id); if (f) sending.delete(f); };
}
await new Promise(() => {});
"#;

/// Start the background pass once per app instance: it sends scheduled
/// drafts when they come due and finishes held sends that a quit or crash
/// interrupted. `ComposeBackground` (or the first compose form) calls it.
pub fn ensure_background(app: AppState) {
    if *BACKGROUND.peek() {
        return;
    }
    *BACKGROUND.write() = true;
    spawn_forever(async move {
        let mut first = true;
        loop {
            let s = store();
            let now = Utc::now();
            let watched = WATCHED.peek().clone();
            match s.overdue_queued_sends(now) {
                Ok(ids) => {
                    let mut sent = 0;
                    for id in ids.into_iter().filter(|id| !watched.contains_key(id)) {
                        match s.release_queued_send(id) {
                            Ok(true) => sent += 1,
                            Ok(false) => {}
                            Err(e) => tracing::warn!("finish held send {id}: {e}"),
                        }
                    }
                    if sent > 0 {
                        flush_outbox();
                        app.bump_mail();
                        app.toast(if first || sent == 1 {
                            "A message you sent before Axentra closed was sent now.".to_string()
                        } else {
                            format!("{sent} messages you sent before Axentra closed were sent now.")
                        });
                    }
                }
                Err(e) => tracing::warn!("held sends: {e}"),
            }
            // Scheduled sends that came due while the app was not running go
            // out now, and the user is told (mail spec 6.6).
            let late = s
                .due_scheduled_drafts(now)
                .unwrap_or_default()
                .into_iter()
                .filter(|id| {
                    let at = s.message(*id).ok().flatten().and_then(|m| m.scheduled_send_at);
                    at.is_some_and(|at| now - at > LATE)
                })
                .count();
            match s.send_due_scheduled(now) {
                Ok(ids) if !ids.is_empty() => {
                    flush_outbox();
                    app.bump_mail();
                    match late {
                        0 => {}
                        1 => {
                            app.toast("A scheduled message was sent now. Axentra wasn't running when it was due.");
                        }
                        n => {
                            app.toast(format!("{n} scheduled messages were sent now. Axentra wasn't running when they were due."));
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("scheduled send: {e}"),
            }
            first = false;
            tokio::time::sleep(StdDuration::from_secs(15)).await;
        }
    });
}

/// Mount once near the app root so scheduled sends go out even when no
/// compose form has been opened. Renders nothing.
#[component]
pub fn ComposeBackground() -> Element {
    let app = crate::state::use_app();
    use_effect(move || ensure_background(app));
    use_future(move || async move {
        let mut page = document::eval(SEND_UNDO_JS);
        while page.recv::<String>().await.is_ok() {
            undo(app, true);
        }
    });
    rsx! {}
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use axentra_core::*;

    use super::*;
    use crate::state::provide_app_state;

    /// Ctrl+Z while "Sending…" shows takes the send back, wherever focus
    /// is: the send is the most recent action, ahead of an older archive.
    /// The next Ctrl+Z undoes the archive; a send that went out is off
    /// the stack, and an undo bar's button undoes its action only once.
    #[test]
    fn ctrl_z_takes_back_the_held_send_before_older_actions() {
        thread_local! {
            static APP: Cell<Option<AppState>> = const { Cell::new(None) };
        }
        fn root() -> Element {
            let app = provide_app_state();
            APP.with(|a| a.set(Some(app)));
            rsx! {}
        }
        crate::state::init(Arc::new(axentra_core::Store::open_in_memory().unwrap()), None);
        let s = store();
        let mut acct = Account { kind: AccountKind::Local, email: "undo-send@axentra.example".into(), ..Default::default() };
        let acct = s.add_account(&mut acct).unwrap();
        let folder = |role| s.folder_by_role(acct, role).unwrap().unwrap().id;
        let (inbox, archive, drafts, outbox) =
            (folder(FolderRole::Inbox), folder(FolderRole::Archive), folder(FolderRole::Drafts), folder(FolderRole::Outbox));
        let mut m = Message { account_id: acct, folder_id: inbox, subject: "Budget".into(), ..Default::default() };
        let old = s.insert_message(&mut m, None).unwrap();
        let in_folder = |id| s.message(id).unwrap().unwrap().folder_id;
        let texts = |app: AppState| app.toasts.peek().iter().map(|t| t.text.clone()).collect::<Vec<_>>();
        let queue = |subject: &str| {
            let mut d = s.new_draft(ComposeKind::New, Some(acct), None).unwrap();
            d.subject = subject.into();
            d.to.push(EmailAddress::new("Priya", "priya@contoso.example"));
            let until = Utc::now() + chrono::Duration::seconds(10);
            (s.queue_send(&mut d, until).unwrap(), until)
        };

        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        let app = APP.with(|a| a.get()).expect("the app state");
        dom.in_runtime(|| {
            let records = s.archive_messages(&[old]).unwrap();
            app.toast_with_undo("Archived", UndoAction::Moves(records));
            let (record, until) = queue("Hello");
            let sent = record.message_id;
            watch(app, record, until, "Hello".into());
            assert_eq!(in_folder(sent), outbox);

            app.undo_last();
            assert_eq!(in_folder(sent), drafts, "the send is taken back first");
            assert_eq!(in_folder(old), archive, "the older archive stays");
            assert!(!texts(app).iter().any(|t| t == "Sending…"));
            app.undo_last();
            assert_eq!(in_folder(old), inbox);
            app.undo_last();
            assert_eq!(texts(app).last().map(String::as_str), Some("Nothing to undo"));

            // Once the hold ends (the message went out), Ctrl+Z leaves it.
            let (record, until) = queue("Later");
            let later = record.message_id;
            let toast = app.toast_undo_send("Sending…", UndoAction::Moves(vec![record]), until);
            app.forget_undo(toast);
            app.undo_last();
            assert_eq!(in_folder(later), outbox);
            assert_eq!(texts(app).last().map(String::as_str), Some("Nothing to undo"));

            // The undo bar's own Undo takes the action off the stack too.
            let records = s.archive_messages(&[old]).unwrap();
            let bar = app.toast_with_undo("Archived", UndoAction::Moves(records));
            app.undo_toast(bar);
            assert_eq!(in_folder(old), inbox);
            app.undo_last();
            assert_eq!(texts(app).last().map(String::as_str), Some("Nothing to undo"));
        });
    }
}
