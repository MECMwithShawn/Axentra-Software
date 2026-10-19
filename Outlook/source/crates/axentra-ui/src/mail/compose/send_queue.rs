//! Messages that wait before going out: the "Undo send" delay and
//! scheduled sends. Both outlive the compose form, so they run as
//! app-lifetime tasks (`spawn_forever`).

use std::collections::HashSet;
use std::time::{Duration as StdDuration, Instant};

use axentra_core::{MessageId, MoveRecord};
use chrono::{DateTime, Utc};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;

use crate::state::{AppState, ComposeRequest, UndoAction, store, sync};

/// Held sends this session is watching (the background pass skips them).
static WATCHED: GlobalSignal<HashSet<MessageId>> = Signal::global(HashSet::new);
/// Whether the background pass runs in this app instance.
static BACKGROUND: GlobalSignal<bool> = Signal::global(|| false);

/// The shell's undo bar disappears after 8 s; renew it before that while a
/// longer "Undo send" delay is still running.
const RENEW_AFTER: StdDuration = StdDuration::from_secs(7);

/// A scheduled send this overdue was missed (the pass runs every 15 s).
const LATE: chrono::TimeDelta = chrono::TimeDelta::minutes(2);

/// Ask the sync engine to deliver what is waiting in Outbox.
pub fn flush_outbox() {
    if let Some(s) = sync() {
        s.send(axentra_sync::Command::FlushOutbox);
    }
}

/// Show "Sending…" with Undo for a held message and send it when the delay
/// ends. Undo (the toast moves the message back to Drafts) reopens it.
pub fn watch(app: AppState, record: MoveRecord, until: DateTime<Utc>, subject: String) {
    let id = record.message_id;
    WATCHED.write().insert(id);
    let mut toast = app.toast_with_undo("Sending…", UndoAction::Moves(vec![record.clone()]));
    spawn_forever(async move {
        let mut shown = Instant::now();
        loop {
            tokio::time::sleep(StdDuration::from_millis(200)).await;
            let s = store();
            let msg = s.message(id).ok().flatten();
            let back_in_drafts = msg.as_ref().is_some_and(|m| m.is_draft && m.folder_id == record.from_folder);
            let still_held = msg.as_ref().is_some_and(|m| m.is_draft && m.folder_id != record.from_folder);
            if back_in_drafts {
                // Undo: reopen the draft exactly as it was.
                let _ = s.unqueue_send(id);
                app.dismiss_toast(toast);
                app.bump_mail();
                app.compose(ComposeRequest { draft: Some(id), ..ComposeRequest::new_message() });
                break;
            }
            if !still_held {
                // Moved or deleted elsewhere: nothing left to send.
                app.dismiss_toast(toast);
                break;
            }
            if Utc::now() >= until {
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
            if shown.elapsed() >= RENEW_AFTER {
                app.dismiss_toast(toast);
                toast = app.toast_with_undo("Sending…", UndoAction::Moves(vec![record.clone()]));
                shown = Instant::now();
            }
        }
        WATCHED.write().remove(&id);
    });
}

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
                    for id in ids.into_iter().filter(|id| !watched.contains(id)) {
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
    rsx! {}
}
