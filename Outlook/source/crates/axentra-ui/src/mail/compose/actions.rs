//! What a compose form does: save, close, send (validation, undo send,
//! schedule), discard and pop out.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::Instant;

use axentra_core::{Draft, MessageId};
use chrono::{Duration, Local, Utc};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use serde::Deserialize;

use super::address;
use super::send_queue;
use super::state::{Body, ComposeDialog, Field, Form, SaveStatus, SendOpts};
use crate::state::{AppState, ComposeRequest, UndoAction, store};

/// Default message size limit (mail spec 6.8: 25 MB unless the server says otherwise).
pub const SIZE_LIMIT: i64 = 25 * 1024 * 1024;

/// The body as the editor has it: HTML, and the new text only (no
/// signature or quoted history).
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Snapshot {
    html: String,
    text: String,
}

/// JS that reads body editor `id`, after keys still arriving have landed
/// when `wait_keys`. The editor is looked up first: if the form goes while
/// it waits, the editor is detached but still holds the text.
pub fn snapshot_js(id: &str, wait_keys: bool) -> String {
    let wait = if wait_keys { "if (window.axTypedText && axTypedText.quiet) await axTypedText.quiet(); " } else { "" };
    format!("const ed = window.axComposeEditors && window.axComposeEditors[{id:?}]; {wait}return ed ? ed.snapshot() : null;")
}

/// True when the editor's `fresh` body is news to the app, which last knew
/// `known`: other HTML and, for a draft never saved, some text (an empty
/// form is not worth a draft).
pub fn body_differs(fresh: &Snapshot, known: &str, saved: bool) -> bool {
    fresh.html != known && (saved || !fresh.text.trim().is_empty())
}

pub fn set_body(d: &mut Draft, s: Snapshot) {
    d.body_html = s.html;
}

fn apply_body(form: Form, s: Snapshot) {
    let mut body = form.body;
    if body.peek().html != s.html {
        body.set(Body { html: s.html, text: s.text });
        form.touch();
    } else {
        body.write().text = s.text;
    }
}

/// Pull the body from the editor now (the editor also pushes debounced
/// snapshots; this makes sure the last keystrokes are included). Keys that
/// are still arriving (queued behind the click or shortcut that started the
/// action, on a busy machine) land first, in the field they were typed in,
/// so the action reads them and they don't spill into what comes next.
pub async fn refresh_body(form: Form) {
    if let Ok(Some(s)) = document::eval(&snapshot_js(&form.dom("body"), true)).join::<Option<Snapshot>>().await {
        apply_body(form, s);
    }
}

/// A send between Send (Ctrl+Enter, Alt+S) and the moment its body has been
/// read. The body is read by an app-lifetime task, not the form's: when the
/// form goes first (a module shortcut pressed right after Ctrl+Enter
/// unmounts it), the send is handed over ([`hand_over_send`]) and still goes
/// out, instead of vanishing with the form while the draft keeps an old body.
#[derive(Default)]
pub struct SendHandoff {
    /// The body as read (the inner `None`: no editor answered).
    body: Option<Option<Snapshot>>,
    /// Set by a form that went before the body came: finish without it.
    orphan: Option<Orphan>,
    waker: Option<Waker>,
    /// Ctrl+Z came before the send went ([`take_back_starting_send`]).
    taken_back: bool,
}

pub type Handoff = Rc<RefCell<SendHandoff>>;

/// A send to finish without its form, with the body read for it and
/// whether Ctrl+Z took it back meanwhile.
type Finish = (Orphan, Option<Snapshot>, bool);

thread_local! {
    /// Sends still reading their body, newest last (a send leaves once it
    /// has gone out, or was refused).
    static STARTING: RefCell<Vec<std::rc::Weak<RefCell<SendHandoff>>>> = const { RefCell::new(Vec::new()) };
}

/// Ctrl+Z right after Send, before the message has been queued: the send
/// is the most recent action, so it is taken back (the form stays open,
/// or the draft reopens when the form has gone). False when no send is
/// starting.
pub fn take_back_starting_send() -> bool {
    STARTING.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|w| w.strong_count() > 0);
        let newest = s.last().and_then(|w| w.upgrade());
        newest.inspect(|h| h.borrow_mut().taken_back = true).is_some()
    })
}

impl SendHandoff {
    /// The reading task has the body: give it to the waiting form, or
    /// return the send to finish when the form has gone.
    fn body_read(h: &Handoff, body: Option<Snapshot>) -> Option<Finish> {
        let mut h = h.borrow_mut();
        if let Some(job) = h.orphan.take() {
            return Some((job, body, h.taken_back));
        }
        h.body = Some(body);
        if let Some(w) = h.waker.take() {
            w.wake();
        }
        None
    }

    /// The form is going: the send to finish now when its body was read but
    /// not taken yet; otherwise the reading task finishes it.
    fn form_gone(h: &Handoff, job: Orphan) -> Option<Finish> {
        let mut h = h.borrow_mut();
        match h.body.take() {
            Some(body) => Some((job, body, h.taken_back)),
            None => {
                h.orphan = Some(job);
                None
            }
        }
    }
}

/// A send whose form has gone: what it had when it went.
pub struct Orphan {
    draft: Draft,
    /// The new text the app had (used when the editor can't be read).
    text: String,
    opts: SendOpts,
    /// The open compose request then (reopens the form when coming back to Mail).
    request: Option<ComposeRequest>,
}

/// Resolves with the body once the reading task has it.
struct BodyRead(Handoff);

impl Future for BodyRead {
    type Output = Option<Snapshot>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut h = self.0.borrow_mut();
        match h.body.take() {
            Some(body) => Poll::Ready(body),
            None => {
                h.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// Read the body for a send, in a task that outlives the form.
fn read_body_for_send(app: AppState, form: Form) -> BodyRead {
    let handoff: Handoff = Rc::default();
    STARTING.with(|s| s.borrow_mut().push(Rc::downgrade(&handoff)));
    let mut slot = form.send_handoff;
    slot.set(Some(handoff.clone()));
    let eval = document::eval(&snapshot_js(&form.dom("body"), true));
    let h = handoff.clone();
    spawn_forever(async move {
        let body = eval.join::<Option<Snapshot>>().await.ok().flatten();
        if let Some((job, body, taken_back)) = SendHandoff::body_read(&h, body) {
            send_without_form(app, job, body, taken_back);
        }
    });
    BodyRead(handoff)
}

/// The form is going (unmounted) with a send still waiting for its body:
/// hand the send over so it finishes without the form. True when there was
/// one (the form then has nothing else to save).
pub fn hand_over_send(app: AppState, form: Form) -> bool {
    let Some(h) = form.send_handoff.try_peek().ok().and_then(|s| s.clone()) else { return false };
    let (Ok(mut draft), Ok(body), Ok(opts)) =
        (form.draft.try_peek().map(|d| d.clone()), form.body.try_peek().map(|b| b.clone()), form.send_opts.try_peek().map(|o| *o))
    else {
        return false;
    };
    draft.body_html = body.html;
    // An address typed after the last chip goes too (see `send`).
    super::recipients::add_typed_on_drop(form, &mut draft);
    let request = app.compose.try_peek().ok().and_then(|r| r.clone());
    let job = Orphan { draft, text: body.text, opts, request };
    if let Some((job, body, taken_back)) = SendHandoff::form_gone(&h, job) {
        send_without_form(app, job, body, taken_back);
    }
    true
}

/// Finish a send whose form has gone: it goes out as it would have from the
/// form. One that needed the user there (no recipient, no subject, a
/// missing attachment, ...) is saved to Drafts instead, with a message
/// saying why it wasn't sent. One that Ctrl+Z took back is saved and
/// reopened, as Undo send does.
fn send_without_form(app: AppState, job: Orphan, body: Option<Snapshot>, taken_back: bool) {
    let Orphan { mut draft, text, opts, request } = job;
    let text = match body {
        Some(s) => {
            draft.body_html = s.html;
            s.text
        }
        None => text,
    };
    if taken_back {
        if let Some(id) = app.check(store().save_draft(&mut draft)) {
            app.bump_mail();
            app.toast("Undone");
            app.compose(ComposeRequest { draft: Some(id), ..ComposeRequest::new_message() });
        }
        return;
    }
    let remind = app.settings.peek().attachment_reminder;
    let subject = subject_or_default(&draft);
    match problem(&draft, &text, opts, remind) {
        None => match go_out(app, &mut draft, opts.schedule) {
            Ok(()) => {
                app.bump_mail();
                // Coming back to Mail must not reopen what has gone out.
                let mut compose = app.compose;
                if request.is_some() && compose.try_peek().is_ok_and(|c| *c == request) {
                    compose.set(None);
                }
                return;
            }
            Err(()) => {
                app.error(format!("Couldn't send \"{subject}\". Saved in Drafts."));
            }
        },
        Some(p) => {
            app.error(format!("\"{subject}\" wasn't sent: {}. It's in Drafts.", p.reason()));
        }
    }
    match store().save_draft(&mut draft) {
        Ok(id) => {
            app.bump_mail();
            point_request_at(app, request, id);
        }
        Err(e) => {
            app.error(format!("Couldn't save your draft: {e}"));
        }
    }
}

/// The open compose request, if it is still `request`, now opens draft
/// `id` (saved for the first time after its form went).
pub fn point_request_at(app: AppState, request: Option<ComposeRequest>, id: MessageId) {
    let mut compose = app.compose;
    if request.is_some()
        && compose.try_peek().is_ok_and(|c| *c == request)
        && let Ok(mut c) = compose.try_write()
        && let Some(r) = c.as_mut()
    {
        r.draft = Some(id);
    }
}

fn subject_or_default(d: &Draft) -> String {
    if d.subject.trim().is_empty() { "(No subject)".to_string() } else { d.subject.clone() }
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
/// is saved to Drafts first, an address still being typed included.
pub async fn close(app: AppState, form: Form) {
    refresh_body(form).await;
    super::recipients::commit_typed(form);
    if form.is_dirty() && (has_content(form) || form.draft.peek().id.is_some()) && save_now(app, form).is_none() {
        app.error("Couldn't save draft. Your changes are still open.");
        return;
    }
    let mut closed = form.closed;
    closed.set(true);
    leave(app);
}

/// Close the form's window. A popped-out form gives focus back to the
/// message list, as the read window does (Esc, its X); an inline one hands
/// it to the reading pane it was in.
fn leave(app: AppState) {
    let popped = app.compose.peek().as_ref().is_some_and(|c| c.popout);
    let mut compose = app.compose;
    compose.set(None);
    if popped && let Some(ctx) = try_consume_context::<crate::mail::ctx::MailCtx>() {
        ctx.request_list_focus();
    }
}

/// Discard: closes at once when nothing was written and nothing saved,
/// otherwise asks first.
pub async fn discard(app: AppState, mut form: Form) {
    refresh_body(form).await;
    if form.draft.peek().id.is_none() && !has_content(form) {
        forget_send(form);
        let mut closed = form.closed;
        closed.set(true);
        leave(app);
        return;
    }
    form.dialog.set(Some(ComposeDialog::Discard));
}

pub fn confirm_discard(app: AppState, form: Form) {
    forget_send(form);
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
    leave(app);
}

/// A discarded form takes a send still waiting for its body with it.
fn forget_send(form: Form) {
    let mut slot = form.send_handoff;
    slot.set(None);
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

/// Why a message can't go out as it stands (mail spec 6.8, in that order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    NoRecipients,
    /// The first address that isn't valid: its well and index.
    InvalidAddress(Field, usize),
    /// No subject, and "Send anyway" not chosen.
    NoSubject,
    /// The text mentions an attachment and there is none (when the
    /// reminder is on and was not dismissed).
    MentionsAttachment,
    TooLarge,
}

impl Problem {
    /// Why it wasn't sent, for a message when the form has gone.
    fn reason(self) -> &'static str {
        match self {
            Problem::NoRecipients => "it has no recipients",
            Problem::InvalidAddress(..) => "an address isn't valid",
            Problem::NoSubject => "it has no subject",
            Problem::MentionsAttachment => "it mentions an attachment but has none",
            Problem::TooLarge => "its attachments are over the size limit",
        }
    }
}

/// The first [`Problem`] with sending `d` (`text`: its new text).
pub fn problem(d: &Draft, text: &str, opts: SendOpts, remind: bool) -> Option<Problem> {
    if d.to.is_empty() && d.cc.is_empty() && d.bcc.is_empty() {
        return Some(Problem::NoRecipients);
    }
    let invalid = [Field::To, Field::Cc, Field::Bcc]
        .into_iter()
        .find_map(|f| f.list(d).iter().position(|a| !address::is_valid(&a.address)).map(|i| (f, i)));
    if let Some((field, index)) = invalid {
        return Some(Problem::InvalidAddress(field, index));
    }
    if d.subject.trim().is_empty() && !opts.skip_subject {
        return Some(Problem::NoSubject);
    }
    if remind && d.attachments.is_empty() && !opts.skip_attachment && mentions_attachment(text) {
        return Some(Problem::MentionsAttachment);
    }
    if attachments_size(d) > SIZE_LIMIT {
        return Some(Problem::TooLarge);
    }
    None
}

/// Send button, Ctrl+Enter, Alt+S and the schedule dialog all end here.
/// Validation follows mail spec 6.8 in order; a dialog that the user
/// confirms sets the matching skip flag and calls this again.
pub async fn send(app: AppState, mut form: Form) {
    if *form.busy.peek() || *form.closed.peek() || form.send_handoff.peek().is_some() {
        return;
    }
    let read = read_body_for_send(app, form);
    let handoff = read.0.clone();
    let body = read.await;
    let mut slot = form.send_handoff;
    slot.set(None);
    if let Some(s) = body {
        apply_body(form, s);
    }
    // An address typed after the last chip goes too (clicking Send adds it
    // as the well loses focus; Ctrl+Enter and Alt+S dropped it).
    super::recipients::commit_typed(form);
    let taken_back = handoff.borrow().taken_back;
    drop(handoff);
    if taken_back {
        // Ctrl+Z while the body was read: nothing went out, the form stays.
        form.send_opts.set(SendOpts::default());
        save_now(app, form);
        app.toast("Undone");
        form.exec("focus", serde_json::Value::Null);
    } else if send_checked(app, form) {
        return;
    }
    // Still in the form: Ctrl+Z there undoes typing again (SEND_UNDO_JS).
    crate::kit::dom::run(&format!("window.__axSendRefused && window.__axSendRefused({})", crate::kit::dom::js_str(&form.dom("root"))));
}

/// Check the form and send it (mail spec 6.8, in that order). False when
/// it stays open: a problem to fix, a question to answer, a failed send.
fn send_checked(app: AppState, mut form: Form) -> bool {
    let opts = *form.send_opts.peek();
    let d = current(form);
    let remind = app.settings.peek().attachment_reminder;
    match problem(&d, &form.body.peek().text, opts, remind) {
        Some(Problem::NoRecipients) => {
            form.send_opts.set(SendOpts::default());
            form.error.set(Some("Add at least one recipient.".into()));
            form.expanded.set(true);
            form.focus(Field::To.key());
            return false;
        }
        Some(Problem::InvalidAddress(field, index)) => {
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
            return false;
        }
        _ => {}
    }
    form.error.set(None);
    match problem(&d, &form.body.peek().text, opts, remind) {
        Some(Problem::NoSubject) => {
            form.dialog.set(Some(ComposeDialog::NoSubject));
            return false;
        }
        Some(Problem::MentionsAttachment) => {
            form.dialog.set(Some(ComposeDialog::AttachReminder));
            return false;
        }
        Some(_) => {
            // Too large: the size banner is already showing; nothing to add.
            form.send_opts.set(SendOpts::default());
            return false;
        }
        None => {}
    }
    form.send_opts.set(SendOpts::default());
    deliver(app, form, d, opts.schedule)
}

fn deliver(app: AppState, mut form: Form, mut d: Draft, schedule: Option<chrono::DateTime<Utc>>) -> bool {
    form.busy.set(true);
    match go_out(app, &mut d, schedule) {
        Ok(()) => {
            let mut closed = form.closed;
            closed.set(true);
            app.bump_mail();
            let mut compose = app.compose;
            compose.set(None);
            true
        }
        Err(()) => {
            form.busy.set(false);
            app.error(format!("Couldn't send \"{}\". Saved in Drafts.", subject_or_default(&d)));
            save_now(app, form);
            false
        }
    }
}

/// Send `d`: scheduled, held for "Undo send", or at once, with the toast
/// that says so.
fn go_out(app: AppState, d: &mut Draft, schedule: Option<chrono::DateTime<Utc>>) -> Result<(), ()> {
    let s = store();
    let subject = subject_or_default(d);
    let result = if let Some(at) = schedule {
        s.schedule_send(d, at).map(|_| {
            let h24 = app.settings.peek().use_24h_time;
            app.toast(format!("Your message will be sent {}.", crate::fmt::full_time(at, h24)));
        })
    } else {
        let delay = app.settings.peek().undo_send_secs.min(30);
        if delay > 0 {
            let until = Utc::now() + Duration::seconds(delay as i64);
            s.queue_send(d, until).map(|record| send_queue::watch(app, record, until, subject))
        } else {
            s.send_draft(d).map(|_| {
                send_queue::flush_outbox();
                app.toast("Sent");
            })
        }
    };
    result.map_err(|e| tracing::warn!("send failed: {e}"))
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

/// Move the form between the reading pane and its own window. The form is
/// mounted afresh there and opens the request's draft, so unsaved work is
/// saved first (the request then points at it), not only when the old form
/// goes away.
pub async fn set_popout(app: AppState, form: Form, popout: bool) {
    refresh_body(form).await;
    // Keys still arriving after this are held and typed into the same field
    // of the new form (kit/typed_text.js), not lost with this one.
    let parts = ["subject", "to", "cc", "bcc"];
    let ids: Vec<String> = parts.iter().map(|p| form.dom(p)).collect();
    let js = format!(
        "const t = window.axTypedText; const id = t && t.typing ? t.typing() : null; \
         if (id && {ids}.includes(id)) {{ t.hold(); return id; }} return null;",
        ids = serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into())
    );
    let focused = document::eval(&js).join::<Option<String>>().await.ok().flatten();
    if let Some(i) = ids.iter().position(|id| focused.as_deref() == Some(id.as_str())) {
        super::state::keep_focus_for_next_form(parts[i]);
    }
    // An address being typed moves with the form, still typed (this form's
    // wells are emptied, so going it doesn't add it here as well).
    let typed = Field::ALL.map(|f| {
        let mut t = form.typed(f);
        let v = t.peek().clone();
        t.set(String::new());
        v
    });
    super::state::keep_typed_for_next_form(typed);
    if form.is_dirty() && (form.draft.peek().id.is_some() || has_content(form)) {
        save_now(app, form);
    }
    let mut compose = app.compose;
    if let Some(r) = compose.write().as_mut() {
        r.popout = popout;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axentra_core::{DraftAttachment, EmailAddress};

    fn snap(text: &str) -> Snapshot {
        Snapshot { html: format!("<div>{text}</div>"), text: text.into() }
    }

    fn orphan(subject: &str) -> Orphan {
        Orphan { draft: Draft { subject: subject.into(), ..Default::default() }, text: String::new(), opts: SendOpts::default(), request: None }
    }

    fn poll(read: &mut BodyRead) -> Poll<Option<Snapshot>> {
        Pin::new(read).poll(&mut Context::from_waker(Waker::noop()))
    }

    /// Ctrl+Enter, then a module shortcut before the body is read: the form
    /// goes, and the send is finished by the reading task with the body it
    /// read, instead of vanishing with the form.
    #[test]
    fn a_send_outlives_its_form() {
        // The form is still there when the body comes: it takes it.
        let h: Handoff = Rc::default();
        let mut read = BodyRead(h.clone());
        assert!(poll(&mut read).is_pending());
        assert!(SendHandoff::body_read(&h, Some(snap("Hello"))).is_none());
        assert_eq!(poll(&mut read), Poll::Ready(Some(snap("Hello"))));

        // The form goes first: the reading task finishes the send.
        let h: Handoff = Rc::default();
        assert!(SendHandoff::form_gone(&h, orphan("Budget")).is_none());
        let (job, body, taken_back) = SendHandoff::body_read(&h, Some(snap("Numbers attached"))).expect("finished without the form");
        assert_eq!(job.draft.subject, "Budget");
        assert_eq!(body, Some(snap("Numbers attached")));
        assert!(!taken_back);

        // The body came but the form went before taking it: not lost.
        let h: Handoff = Rc::default();
        assert!(SendHandoff::body_read(&h, Some(snap("Late"))).is_none());
        let (job, body, _) = SendHandoff::form_gone(&h, orphan("Q4")).expect("finished at once");
        assert_eq!((job.draft.subject.as_str(), body), ("Q4", Some(snap("Late"))));
    }

    /// Ctrl+Z pressed right after Ctrl+Enter, while the body is still being
    /// read: the starting send is taken back (newest first), also one whose
    /// form has gone; once no send is starting, Ctrl+Z is for the undo stack.
    #[test]
    fn ctrl_z_takes_back_a_send_still_starting() {
        let start = || {
            let h: Handoff = Rc::default();
            STARTING.with(|s| s.borrow_mut().push(Rc::downgrade(&h)));
            h
        };
        assert!(!take_back_starting_send(), "no send is starting");
        let older = start();
        let newer = start();
        assert!(take_back_starting_send());
        assert!(newer.borrow().taken_back && !older.borrow().taken_back, "the most recent send");

        // The form went before its body came: the draft is kept, not sent.
        assert!(SendHandoff::form_gone(&newer, orphan("Oops")).is_none());
        let (job, _, taken_back) = SendHandoff::body_read(&newer, Some(snap("Not yet"))).expect("finished without the form");
        assert_eq!((job.draft.subject.as_str(), taken_back), ("Oops", true));

        // A send that went out (its handoff dropped) is not starting any more.
        drop((older, newer));
        assert!(!take_back_starting_send());
    }

    #[test]
    fn send_checks_in_order() {
        let to = |a: &str| vec![EmailAddress::new(String::new(), a.to_string())];
        let ok = Draft { to: to("zed@example.test"), subject: "Hi".into(), ..Default::default() };
        let opts = SendOpts::default();
        assert_eq!(problem(&ok, "Hello", opts, true), None);
        assert_eq!(problem(&Draft { subject: "Hi".into(), ..Default::default() }, "", opts, true), Some(Problem::NoRecipients));
        let bad = Draft { to: to("zed@"), subject: String::new(), ..Default::default() };
        assert_eq!(problem(&bad, "", opts, true), Some(Problem::InvalidAddress(Field::To, 0)));
        let blank = Draft { subject: " ".into(), ..ok.clone() };
        assert_eq!(problem(&blank, "", opts, true), Some(Problem::NoSubject));
        assert_eq!(problem(&blank, "", SendOpts { skip_subject: true, ..opts }, true), None);
        assert_eq!(problem(&ok, "See attached.", opts, true), Some(Problem::MentionsAttachment));
        assert_eq!(problem(&ok, "See attached.", opts, false), None);
        assert_eq!(problem(&ok, "See attached.", SendOpts { skip_attachment: true, ..opts }, true), None);
        let big = Draft { attachments: vec![DraftAttachment { size: SIZE_LIMIT + 1, ..Default::default() }], ..ok.clone() };
        assert_eq!(problem(&big, "", opts, true), Some(Problem::TooLarge));
    }

    #[test]
    fn a_body_read_after_the_form_went_is_saved_only_when_new() {
        let known = "<div>Hello</div>";
        assert!(!body_differs(&snap("Hello"), known, true), "nothing new");
        assert!(body_differs(&snap("Hello team"), known, true));
        assert!(!body_differs(&Snapshot { html: "<div><br></div>".into(), text: "\n".into() }, "", false), "an empty form is not a draft");
        assert!(body_differs(&snap("Typed only here"), "", false));
    }

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
