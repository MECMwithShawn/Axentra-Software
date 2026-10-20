//! What a compose form does: save, close, send (validation, undo send,
//! schedule), discard and pop out.

use std::time::Instant;

use axentra_core::Draft;
use chrono::{Duration, Local, Utc};
use dioxus::prelude::*;
use serde::Deserialize;

use super::address;
use super::send_queue;
use super::state::{Body, ComposeDialog, Field, Form, SaveStatus, SendOpts};
use crate::state::{AppState, ComposeRequest, UndoAction, store};

/// Default message size limit (mail spec 6.8: 25 MB unless the server says otherwise).
pub const SIZE_LIMIT: i64 = 25 * 1024 * 1024;

#[derive(Deserialize)]
struct Snapshot {
    html: String,
    text: String,
}

/// Pull the body from the editor now (the editor also pushes debounced
/// snapshots; this makes sure the last keystrokes are included).
pub async fn refresh_body(form: Form) {
    let id = form.dom("body");
    let js = format!("const ed = window.axComposeEditors && window.axComposeEditors[{id:?}]; return ed ? ed.snapshot() : null;");
    if let Ok(Some(s)) = document::eval(&js).join::<Option<Snapshot>>().await {
        let mut body = form.body;
        if body.peek().html != s.html {
            body.set(Body { html: s.html, text: s.text });
            form.touch();
        } else {
            body.write().text = s.text;
        }
    }
}

/// The draft as it stands, including the latest body.
pub fn current(form: Form) -> Draft {
    let mut d = form.draft.peek().clone();
    d.body_html = form.body.peek().html.clone();
    d
}

/// Save the draft now. Returns its message id.
pub fn save_now(app: AppState, mut form: Form) -> Option<axentra_core::MessageId> {
    if *form.closed.peek() {
        return None;
    }
    let mut d = current(form);
    let first = d.id.is_none();
    match store().save_draft(&mut d) {
        Ok(id) => {
            form.draft.set(d);
            form.saved.set(*form.dirty.peek());
            form.last_save.set(Instant::now());
            form.status.set(SaveStatus::Saved(Local::now()));
            if *form.saved_id.peek() != Some(id) {
                form.saved_id.set(Some(id));
            }
            // Point the open request at the saved draft, so leaving Mail and
            // coming back reopens this draft instead of a blank form.
            if first {
                let mut compose = app.compose;
                if let Some(r) = compose.write().as_mut() {
                    r.draft = Some(id);
                }
            }
            app.bump_mail();
            Some(id)
        }
        Err(e) => {
            form.status.set(SaveStatus::Failed(e.to_string()));
            None
        }
    }
}

/// True when the user put anything into the form.
pub fn has_content(form: Form) -> bool {
    let d = form.draft.peek();
    !d.to.is_empty()
        || !d.cc.is_empty()
        || !d.bcc.is_empty()
        || !d.subject.trim().is_empty()
        || !d.attachments.is_empty()
        || !form.body.peek().text.trim().is_empty()
}

/// Close the form (Esc, close button). Never discards: a form with changes
/// is saved to Drafts first.
pub async fn close(app: AppState, form: Form) {
    refresh_body(form).await;
    if form.is_dirty() && (has_content(form) || form.draft.peek().id.is_some()) && save_now(app, form).is_none() {
        app.error("Couldn't save draft. Your changes are still open.");
        return;
    }
    let mut closed = form.closed;
    closed.set(true);
    let mut compose = app.compose;
    compose.set(None);
}

/// Discard: closes at once when nothing was written and nothing saved,
/// otherwise asks first.
pub async fn discard(app: AppState, mut form: Form) {
    refresh_body(form).await;
    if form.draft.peek().id.is_none() && !has_content(form) {
        let mut closed = form.closed;
        closed.set(true);
        let mut compose = app.compose;
        compose.set(None);
        return;
    }
    form.dialog.set(Some(ComposeDialog::Discard));
}

pub fn confirm_discard(app: AppState, form: Form) {
    let mut closed = form.closed;
    closed.set(true);
    if let Some(id) = form.draft.peek().id {
        match store().discard_draft_undoable(id) {
            Ok(records) => {
                app.bump_mail();
                if !records.is_empty() {
                    app.toast_with_undo("Draft discarded", UndoAction::Moves(records));
                }
            }
            Err(e) => {
                app.error(format!("Couldn't discard the draft: {e}"));
            }
        }
    }
    let mut compose = app.compose;
    compose.set(None);
}

/// Words that suggest a file should be attached. Only the text the user
/// wrote is checked (no signature, no quoted history; mail spec 14.18).
pub fn mentions_attachment(text: &str) -> bool {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if words.iter().any(|w| w.starts_with("attach") || *w == "enclosed" || *w == "enclosure") {
        return true;
    }
    let joined = words.join(" ");
    ["see file", "see the file", "see the files", "file included", "files included", "included file", "included files"]
        .iter()
        .any(|p| joined.contains(p))
}

/// Total size of the attachments.
pub fn attachments_size(d: &Draft) -> i64 {
    d.attachments.iter().map(|a| a.size.max(0)).sum()
}

/// Send button, Ctrl+Enter, Alt+S and the schedule dialog all end here.
/// Validation follows mail spec 6.8 in order; a dialog that the user
/// confirms sets the matching skip flag and calls this again.
pub async fn send(app: AppState, mut form: Form) {
    if *form.busy.peek() || *form.closed.peek() {
        return;
    }
    refresh_body(form).await;
    let opts = *form.send_opts.peek();
    let d = current(form);
    if d.to.is_empty() && d.cc.is_empty() && d.bcc.is_empty() {
        form.send_opts.set(SendOpts::default());
        form.error.set(Some("Add at least one recipient.".into()));
        form.expanded.set(true);
        form.focus(Field::To.key());
        return;
    }
    let invalid = [Field::To, Field::Cc, Field::Bcc]
        .into_iter()
        .find_map(|f| f.list(&d).iter().position(|a| !address::is_valid(&a.address)).map(|i| (f, i)));
    if let Some((field, index)) = invalid {
        form.send_opts.set(SendOpts::default());
        form.error.set(Some("Fix or remove the highlighted addresses.".into()));
        form.expanded.set(true);
        if field == Field::Cc {
            form.show_cc.set(true);
        }
        if field == Field::Bcc {
            form.show_bcc.set(true);
        }
        // The first invalid chip gets focus: Delete removes it, Enter edits it.
        form.chip.set(Some((field, index)));
        form.focus(field.key());
        return;
    }
    form.error.set(None);
    if d.subject.trim().is_empty() && !opts.skip_subject {
        form.dialog.set(Some(ComposeDialog::NoSubject));
        return;
    }
    let remind = app.settings.peek().attachment_reminder;
    if remind && d.attachments.is_empty() && !opts.skip_attachment && mentions_attachment(&form.body.peek().text) {
        form.dialog.set(Some(ComposeDialog::AttachReminder));
        return;
    }
    if attachments_size(&d) > SIZE_LIMIT {
        // The size banner is already showing; nothing to add.
        form.send_opts.set(SendOpts::default());
        return;
    }
    form.send_opts.set(SendOpts::default());
    deliver(app, form, d, opts.schedule);
}

fn deliver(app: AppState, mut form: Form, mut d: Draft, schedule: Option<chrono::DateTime<Utc>>) {
    let s = store();
    form.busy.set(true);
    let subject = if d.subject.trim().is_empty() { "(No subject)".to_string() } else { d.subject.clone() };
    let result = if let Some(at) = schedule {
        s.schedule_send(&mut d, at).map(|_| {
            let h24 = app.settings.peek().use_24h_time;
            app.toast(format!("Your message will be sent {}.", crate::fmt::full_time(at, h24)));
        })
    } else {
        let delay = app.settings.peek().undo_send_secs.min(30);
        if delay > 0 {
            let until = Utc::now() + Duration::seconds(delay as i64);
            s.queue_send(&mut d, until).map(|record| send_queue::watch(app, record, until, subject.clone()))
        } else {
            s.send_draft(&mut d).map(|_| {
                send_queue::flush_outbox();
                app.toast("Sent");
            })
        }
    };
    match result {
        Ok(()) => {
            let mut closed = form.closed;
            closed.set(true);
            app.bump_mail();
            let mut compose = app.compose;
            compose.set(None);
        }
        Err(e) => {
            form.busy.set(false);
            tracing::warn!("send failed: {e}");
            app.error(format!("Couldn't send \"{subject}\". Saved in Drafts."));
            save_now(app, form);
        }
    }
}

/// "Forward as attachment" (message context menu): a Forward draft with
/// the original attached as an `.eml` file, opened in the compose form.
pub fn forward_as_attachment(app: AppState, source: axentra_core::MessageId) {
    match store().new_forward_as_attachment(source) {
        Ok(d) => {
            app.bump_mail();
            app.compose(ComposeRequest { draft: d.id, ..ComposeRequest::new_message() });
        }
        Err(e) => {
            app.error(format!("Couldn't forward the message as an attachment: {e}"));
        }
    }
}

/// Move the form between the reading pane and its own window.
pub async fn set_popout(app: AppState, form: Form, popout: bool) {
    refresh_body(form).await;
    let mut compose = app.compose;
    if let Some(r) = compose.write().as_mut() {
        r.popout = popout;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_words() {
        assert!(mentions_attachment("I've attached the plan."));
        assert!(mentions_attachment("See the attachment"));
        assert!(mentions_attachment("Please find enclosed"));
        assert!(mentions_attachment("see file below"));
        assert!(mentions_attachment("Budget file included."));
        assert!(!mentions_attachment("Lunch tomorrow?"));
        assert!(!mentions_attachment("I included you on the thread"));
        assert!(!mentions_attachment(""));
    }
}
