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
        }
    }

    /// Run an action owned by the form, not by the control that started it:
    /// menus and dialogs close (and drop their tasks) before it finishes.
    pub fn spawn(&self, fut: impl std::future::Future<Output = ()> + 'static) {
        dioxus::core::Runtime::current().spawn(self.scope, fut);
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

    /// Move keyboard focus to an element of this form.
    pub fn focus(&self, part: &str) {
        let id = self.dom(part);
        let js = format!("const e = document.getElementById({id:?}); if (e) {{ e.focus(); }}");
        document::eval(&js);
    }
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
