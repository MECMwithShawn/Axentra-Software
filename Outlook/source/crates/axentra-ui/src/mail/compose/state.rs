//! State of one open compose form, shared with its parts through context,
//! plus the ribbon state the Mail ribbon reads through `ribbon_tabs()`.

use std::time::Instant;

use axentra_core::{Draft, DraftAttachment, EmailAddress, Importance, MessageId};
use chrono::{DateTime, Local};
use dioxus::prelude::*;

use super::format::FormatState;

/// A recipient well.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    To,
    Cc,
    Bcc,
}

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Field::To => "To",
            Field::Cc => "Cc",
            Field::Bcc => "Bcc",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Field::To => "to",
            Field::Cc => "cc",
            Field::Bcc => "bcc",
        }
    }

    pub const ALL: [Field; 3] = [Field::To, Field::Cc, Field::Bcc];

    fn index(self) -> usize {
        match self {
            Field::To => 0,
            Field::Cc => 1,
            Field::Bcc => 2,
        }
    }

    pub fn list(self, d: &Draft) -> &Vec<EmailAddress> {
        match self {
            Field::To => &d.to,
            Field::Cc => &d.cc,
            Field::Bcc => &d.bcc,
        }
    }

    pub fn list_mut(self, d: &mut Draft) -> &mut Vec<EmailAddress> {
        match self {
            Field::To => &mut d.to,
            Field::Cc => &mut d.cc,
            Field::Bcc => &mut d.bcc,
        }
    }
}

/// The footer status text.
#[derive(Clone, Debug, PartialEq)]
pub enum SaveStatus {
    Idle,
    Saved(DateTime<Local>),
    Failed(String),
}

/// Modal dialogs the form can show.
#[derive(Clone, Debug, PartialEq)]
pub enum ComposeDialog {
    Discard,
    NoSubject,
    AttachReminder,
    Schedule,
    Link { text: String, url: String, editing: bool },
    AddressBook(Field),
    TableSize,
    /// Drawing canvas, opened with this tool and ink colour.
    Draw { tool: super::draw::Tool, color: String },
    /// Preview of attachment `index`.
    Preview(usize),
}

/// Flyouts anchored under a ribbon command (viewport position).
#[derive(Clone, Debug, PartialEq)]
pub enum Flyout {
    Table,
    Emoji,
    Palette { highlight: bool },
}

/// Latest body content reported by the editor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Body {
    pub html: String,
    /// Text the user wrote (no signature, no quoted history).
    pub text: String,
}

/// An "@" mention being typed in the body: the text after "@" and where
/// the suggestion list opens (viewport position under the caret).
#[derive(Clone, Debug, PartialEq)]
pub struct Mention {
    pub query: String,
    pub x: f64,
    pub y: f64,
    /// Highlighted suggestion.
    pub hi: usize,
}

/// Choices that skip a validation step on the next send attempt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SendOpts {
    pub skip_subject: bool,
    pub skip_attachment: bool,
    pub schedule: Option<DateTime<chrono::Utc>>,
}

/// Everything one compose form needs; `Copy` so handlers can capture it.
#[derive(Clone, Copy)]
pub struct Form {
    pub uid: u64,
    /// The form component's scope: actions run there (see [`Form::spawn`]).
    pub scope: ScopeId,
    pub draft: Signal<Draft>,
    pub body: CopyValue<Body>,
    /// Bumped on every edit; `saved` holds the value at the last save.
    pub dirty: Signal<u64>,
    pub saved: Signal<u64>,
    pub last_edit: CopyValue<Instant>,
    pub last_save: CopyValue<Instant>,
    pub status: Signal<SaveStatus>,
    pub show_cc: Signal<bool>,
    pub show_bcc: Signal<bool>,
    pub show_from: Signal<bool>,
    /// Inline replies start with a one-line header; this shows all fields.
    pub expanded: Signal<bool>,
    /// The selected recipient chip (one at a time across the wells).
    pub chip: Signal<Option<(Field, usize)>>,
    /// What is typed in each well and not a chip yet ([`Form::typed`]).
    typed: [Signal<String>; 3],
    /// Polite screen-reader announcement ("Added Priya Raman").
    pub live: Signal<String>,
    pub mention: Signal<Option<Mention>>,
    /// Inline error under the To row ("Add at least one recipient.").
    pub error: Signal<Option<String>>,
    pub dialog: Signal<Option<ComposeDialog>>,
    pub flyout: Signal<Option<(Flyout, f64, f64)>>,
    pub fmt: Signal<FormatState>,
    pub busy: Signal<bool>,
    pub color: Signal<String>,
    pub highlight: Signal<String>,
    pub send_opts: Signal<SendOpts>,
    /// Shared with the outer `ComposePane`: the id this form saved to.
    pub saved_id: CopyValue<Option<MessageId>>,
    /// Set once the form was sent or discarded: nothing more to save.
    pub closed: CopyValue<bool>,
    /// A send waiting for its body (see [`super::actions::SendHandoff`]).
    pub send_handoff: CopyValue<Option<super::actions::Handoff>>,
}

impl Form {
    /// Create the form's state in the current (form) component.
    pub fn new(uid: u64, draft: Draft, saved_id: CopyValue<Option<MessageId>>, expanded: bool) -> Self {
        let show_cc = !draft.cc.is_empty();
        let show_bcc = !draft.bcc.is_empty();
        let body = Body { html: draft.body_html.clone(), text: String::new() };
        Form {
            uid,
            scope: dioxus::core::current_scope_id(),
            draft: Signal::new(draft),
            body: CopyValue::new(body),
            dirty: Signal::new(0),
            saved: Signal::new(0),
            last_edit: CopyValue::new(Instant::now()),
            last_save: CopyValue::new(Instant::now()),
            status: Signal::new(SaveStatus::Idle),
            show_cc: Signal::new(show_cc),
            show_bcc: Signal::new(show_bcc),
            show_from: Signal::new(false),
            expanded: Signal::new(expanded),
            chip: Signal::new(None),
            typed: take_typed_for_next_form().map(Signal::new),
            live: Signal::new(String::new()),
            mention: Signal::new(None),
            error: Signal::new(None),
            dialog: Signal::new(None),
            flyout: Signal::new(None),
            fmt: Signal::new(FormatState::default()),
            busy: Signal::new(false),
            color: Signal::new("#C00000".into()),
            highlight: Signal::new("#FFFF00".into()),
            send_opts: Signal::new(SendOpts::default()),
            saved_id,
            closed: CopyValue::new(false),
            send_handoff: CopyValue::new(None),
        }
    }

    /// Run an action owned by the form, not by the control that started it:
    /// menus and dialogs close (and drop their tasks) before it finishes.
    /// The form closing drops it, so it never resumes on a form that has
    /// gone (see [`crate::kit::spawn_in`]).
    pub fn spawn(&self, fut: impl std::future::Future<Output = ()> + 'static) {
        crate::kit::spawn_in(self.scope, self.closed, fut);
    }

    /// The text typed in `field`'s well that is not a chip yet. Send, Esc,
    /// Ctrl+S and leaving the form turn it into recipients
    /// (`recipients::commit_typed`), as leaving the well does.
    pub fn typed(&self, field: Field) -> Signal<String> {
        self.typed[field.index()]
    }

    pub fn dom(&self, part: &str) -> String {
        format!("ax-cmp{}-{part}", self.uid)
    }

    pub fn is_dirty(&self) -> bool {
        *self.dirty.peek() != *self.saved.peek()
    }

    /// Record a user edit (feeds autosave).
    pub fn touch(mut self) {
        self.dirty += 1;
        self.last_edit.set(Instant::now());
    }

    /// Run a command in the body editor (see `editor.js`).
    pub fn exec(&self, cmd: &str, val: serde_json::Value) {
        let id = self.dom("body");
        let js = format!(
            "const ed = window.axComposeEditors && window.axComposeEditors[{}]; if (ed) {{ ed.run({}, {}); }}",
            serde_json::Value::String(id),
            serde_json::Value::String(cmd.to_string()),
            val
        );
        document::eval(&js);
    }

    /// Say something to screen readers (polite).
    pub fn announce(&self, text: impl Into<String>) {
        let mut live = self.live;
        live.set(text.into());
    }

    /// Move keyboard focus to an element of this form. One that comes with
    /// the next render (the Cc line the Cc button just showed) is waited
    /// for a moment, so typing straight on lands in it.
    pub fn focus(&self, part: &str) {
        let id = self.dom(part);
        let js = format!(
            "const f = (n) => {{ const e = document.getElementById({id:?}); if (e) e.focus(); else if (n > 0) setTimeout(() => f(n - 1), 20); }}; f(25);"
        );
        document::eval(&js);
    }

    /// Focus a text field of this form with the caret after its text, and
    /// type there what the page held while the form was replaced
    /// (`axTypedText.hold()`).
    pub fn focus_end(&self, part: &str) {
        let id = self.dom(part);
        let js = format!(
            "const e = document.getElementById({id:?}); \
             if (window.axTypedText && axTypedText.release) axTypedText.release({id:?}); \
             else if (e) {{ e.focus(); const n = e.value.length; try {{ e.setSelectionRange(n, n); }} catch (_) {{}} }}"
        );
        document::eval(&js);
    }
}

/// The field that had focus when the form popped out or back in, for the
/// form mounted there next (keys still arriving keep going into it).
static NEXT_FORM_FOCUS: std::sync::Mutex<Option<(&'static str, Instant)>> = std::sync::Mutex::new(None);

pub fn keep_focus_for_next_form(part: &'static str) {
    if let Ok(mut f) = NEXT_FORM_FOCUS.lock() {
        *f = Some((part, Instant::now()));
    }
}

/// Taken once, by the next form, within a few seconds.
pub fn take_focus_for_next_form() -> Option<&'static str> {
    let taken = NEXT_FORM_FOCUS.lock().ok()?.take()?;
    (taken.1.elapsed() < std::time::Duration::from_secs(5)).then_some(taken.0)
}

/// What was typed in the To, Cc and Bcc wells (not chips yet) when the
/// form popped out or back in: the next form's wells start with it.
static NEXT_FORM_TYPED: std::sync::Mutex<Option<([String; 3], Instant)>> = std::sync::Mutex::new(None);

pub fn keep_typed_for_next_form(typed: [String; 3]) {
    if let Ok(mut t) = NEXT_FORM_TYPED.lock() {
        *t = typed.iter().any(|t| !t.is_empty()).then(|| (typed, Instant::now()));
    }
}

fn take_typed_for_next_form() -> [String; 3] {
    NEXT_FORM_TYPED
        .lock()
        .ok()
        .and_then(|mut t| t.take())
        .filter(|(_, at)| at.elapsed() < std::time::Duration::from_secs(5))
        .map(|(t, _)| t)
        .unwrap_or_default()
}

/// What the ribbon needs to know about the open form. The Mail ribbon
/// calls `ribbon_tabs()` while rendering, which reads this signal, so it
/// re-renders when the selection's formatting or a toggle changes.
#[derive(Clone, Debug, PartialEq)]
pub struct RibbonState {
    pub fmt: FormatState,
    pub importance: Importance,
    pub show_from: bool,
    /// From is always shown with several accounts (Show From is moot).
    pub from_locked: bool,
    pub show_bcc: bool,
    pub read_receipt: bool,
    pub delivery_receipt: bool,
    pub plain_text: bool,
    pub signature: bool,
    pub color: String,
    pub highlight: String,
    pub scheduled: bool,
    /// "Recent files" under Attach file.
    pub recent: Vec<DraftAttachment>,
}

impl Default for RibbonState {
    fn default() -> Self {
        RibbonState {
            fmt: FormatState { font: "Aptos".into(), size: 12.0, align: "left".into(), block: "div".into(), ..Default::default() },
            importance: Importance::Normal,
            show_from: false,
            from_locked: false,
            show_bcc: false,
            read_receipt: false,
            delivery_receipt: false,
            plain_text: false,
            signature: false,
            color: "#C00000".into(),
            highlight: "#FFFF00".into(),
            scheduled: false,
            recent: Vec::new(),
        }
    }
}

pub static RIBBON: GlobalSignal<RibbonState> = Signal::global(RibbonState::default);
