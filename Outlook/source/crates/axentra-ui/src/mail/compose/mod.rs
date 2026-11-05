//! Compose / reply / forward: the form shown in the reading pane (or popped
//! out), its ribbon tabs, drafts and sending (mail spec section 6).
//!
//! * [`ComposePane`] renders the form for a `ComposeRequest`; the Mail
//!   module shows it while `app.compose` is set.
//! * [`ribbon_tabs`] gives the Message / Insert / Format text / Options tabs
//!   for the Mail ribbon. Their command ids start with `compose.`; forward
//!   picks with `app.compose_command(id)`.
//! * [`ComposeBackground`] (mount once near the root) sends scheduled
//!   drafts on time and finishes "Undo send" delays a quit interrupted.
//! * [`forward_as_attachment`] starts a forward with the message attached
//!   as an `.eml` file (for the message context menu).
//! * [`open_outside_requests`] opens new messages asked for from outside
//!   the app (`--compose`, `mailto:` links) as drafts to review.

mod actions;
pub mod address;
mod attachments;
mod commands;
mod dialogs;
mod draw;
mod editor;
mod emoji;
mod flyouts;
mod footer;
mod format;
mod header;
mod mention;
mod pane;
mod recipients;
mod ribbon;
mod schedule;
pub mod send_queue;
mod state;

pub use actions::forward_as_attachment;
pub use pane::ComposePane;
pub use ribbon::ribbon_tabs;
pub use send_queue::ComposeBackground;

/// Open, as drafts for the user to review, the new messages other programs
/// asked for (`axentra_core::compose_request`); `--send` requests are sent
/// instead ([`send_outside_request`]). Each becomes a saved draft
/// in Drafts first, so closing the form keeps it; nothing is sent. Files
/// that could not be attached are said in an error toast.
pub fn open_outside_requests(app: crate::state::AppState) {
    let s = crate::state::store();
    let Some(db) = s.path().map(|p| p.to_path_buf()) else { return };
    for req in axentra_core::compose_request::take(&db) {
        if req.send {
            send_outside_request(app, &db, req);
            continue;
        }
        match s.draft_from_request(&req, actions::SIZE_LIMIT) {
            Ok((draft, problems)) => {
                for p in problems {
                    app.error(p);
                }
                if let Some(id) = draft.id {
                    tracing::debug!(%id, "compose: opening a draft asked for from outside");
                    app.bump_mail();
                    app.compose(crate::state::ComposeRequest { draft: Some(id), ..crate::state::ComposeRequest::new_message() });
                }
            }
            Err(e) => {
                app.error(format!("Couldn't start the new message: {e}"));
            }
        }
    }
}

/// How long a `--send` message may sit in Outbox (retrying) before the
/// command is told it hasn't gone yet.
const SEND_WAIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Send a message asked for with `axentra-outlook --compose --send`, then
/// tell the waiting command how it went. The user approved the exact
/// message before the command ran, so there is no Undo-send delay. A
/// message that can't go (a file couldn't be attached, the server refused
/// it) stays in Drafts.
fn send_outside_request(app: crate::state::AppState, db: &std::path::Path, req: axentra_core::compose_request::ComposeRequest) {
    use axentra_core::compose_request::{SendResult, send_problems, write_result};
    let answer = {
        let db = db.to_path_buf();
        let id = req.id.clone();
        move |sent: bool, text: String| {
            if let Err(e) = write_result(&db, &id, &SendResult { sent, text }) {
                tracing::warn!("send request {id}: couldn't answer: {e}");
            }
        }
    };
    let problems = send_problems(&req);
    if !problems.is_empty() {
        answer(false, format!("Not sent: {}.", problems.join("; ")));
        return;
    }
    let s = crate::state::store();
    let mut draft = match s.draft_from_request(&req, actions::SIZE_LIMIT) {
        Ok((d, problems)) if problems.is_empty() => d,
        Ok((_, problems)) => {
            app.bump_mail();
            answer(false, format!("Not sent; saved in Drafts. {}", problems.join(" ")));
            return;
        }
        Err(e) => {
            answer(false, format!("Not sent: {e}."));
            return;
        }
    };
    let summary = send_summary(&draft);
    let id = match s.send_draft(&mut draft) {
        Ok(id) => id,
        Err(e) => {
            app.bump_mail();
            answer(false, format!("Not sent; saved in Drafts: {e}."));
            return;
        }
    };
    app.bump_mail();
    if let Some(sync) = crate::state::sync() {
        sync.send(axentra_sync::Command::FlushOutbox);
    }
    // Follow the message until it is in Sent Items (or back in Drafts).
    dioxus::core::spawn_forever(async move {
        let started = std::time::Instant::now();
        loop {
            let s = crate::state::store();
            let m = s.message(id).ok().flatten();
            let role = m.as_ref().and_then(|m| s.folder(m.folder_id).ok().flatten()).map(|f| f.role);
            let error = crate::state::sync().and_then(|h| h.send_error(id));
            match role {
                Some(axentra_core::FolderRole::Sent) => {
                    let mid = m.map(|m| m.internet_message_id).filter(|i| !i.is_empty()).map(|i| format!(", Message-ID <{}>", i.trim_matches(['<', '>']))).unwrap_or_default();
                    app.toast(format!("Sent from the command line: {summary}"));
                    answer(true, format!("Sent {summary}; in Sent Items{mid}."));
                    return;
                }
                Some(axentra_core::FolderRole::Drafts) | None => {
                    let why = error.map(|(e, _)| e).unwrap_or_else(|| "it was taken back".into());
                    answer(false, format!("Not sent; saved in Drafts: {why}"));
                    return;
                }
                _ if started.elapsed() > SEND_WAIT => {
                    let why = error.map(|(e, _)| format!(" Last error: {e}")).unwrap_or_default();
                    answer(false, format!("Not sent yet: still in Outbox; Axentra Outlook will keep trying.{why}"));
                    return;
                }
                _ => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
            }
        }
    });
}

/// "to a@x, b@y: “Subject” with Guide.zip (6.9 MB)".
fn send_summary(d: &axentra_core::Draft) -> String {
    let who: Vec<&str> = d.to.iter().chain(&d.cc).chain(&d.bcc).map(|a| a.address.as_str()).collect();
    let files: Vec<String> = d.attachments.iter().map(|a| format!("{} ({})", a.name, crate::fmt::file_size(a.size))).collect();
    let with = if files.is_empty() { String::new() } else { format!(" with {}", files.join(", ")) };
    format!("to {}: \u{201c}{}\u{201d}{with}", who.join(", "), d.subject)
}
