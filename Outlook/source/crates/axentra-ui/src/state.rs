//! App-wide state shared through Dioxus context.
//!
//! The store itself is not reactive. Components subscribe to a domain by
//! reading its revision signal (e.g. `app.mail_rev()`) before querying the
//! store, and every mutation is followed by the matching `bump_*` call.

use std::sync::{Arc, OnceLock};

use axentra_core::*;
use axentra_core::{Event, Result, Store};
use dioxus::prelude::*;

static STORE: OnceLock<Arc<Store>> = OnceLock::new();
static SYNC: OnceLock<axentra_sync::SyncHandle> = OnceLock::new();

/// Install the store (and optionally the sync engine) before launching the UI.
pub fn init(store: Arc<Store>, sync: Option<axentra_sync::SyncHandle>) {
    let _ = STORE.set(store);
    if let Some(s) = sync {
        let _ = SYNC.set(s);
    }
}

/// The process-wide store. Panics if [`init`] was not called.
pub fn store() -> &'static Store {
    STORE.get().expect("axentra_ui::init must be called before launching").as_ref()
}

pub fn store_arc() -> Arc<Store> {
    STORE.get().expect("axentra_ui::init must be called before launching").clone()
}

/// The sync engine, when running (absent in the preview server and tests).
pub fn sync() -> Option<&'static axentra_sync::SyncHandle> {
    SYNC.get()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Module {
    #[default]
    Mail,
    Calendar,
    People,
    Tasks,
}

impl Module {
    pub const ALL: [Module; 4] = [Module::Mail, Module::Calendar, Module::People, Module::Tasks];

    pub fn label(self) -> &'static str {
        match self {
            Module::Mail => "Mail",
            Module::Calendar => "Calendar",
            Module::People => "People",
            Module::Tasks => "To Do",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Module::Mail => "mail",
            Module::Calendar => "calendar_ltr",
            Module::People => "people",
            Module::Tasks => "checkmark_circle",
        }
    }
}

/// What an undo button in a toast reverses.
#[derive(Clone, Debug, PartialEq)]
pub enum UndoAction {
    /// Move messages back (after delete / move / archive / junk).
    Moves(Vec<MoveRecord>),
    /// Re-insert a deleted task / event / contact (inserted with a new id).
    RestoreTask(Task),
    RestoreEvent(Event),
    RestoreContact(Contact),
    /// Undo a message flag/read/pin change by writing the old messages back.
    RestoreMessages(Vec<Message>),
}

/// What a custom toast button does (besides Undo).
#[derive(Clone, Debug, PartialEq)]
pub enum ToastAction {
    /// Open a file or folder in the desktop's default app (`xdg-open`),
    /// e.g. "Open folder" after an export.
    OpenPath(std::path::PathBuf),
    /// Go somewhere in the app.
    Navigate(NavRequest),
    /// Open the Settings dialog on a page.
    OpenSettings(SettingsPage),
}

/// A countdown on a toast ("Sending…" while Undo send waits): the toast
/// shows a spinner and a draining bar and closes when `until` passes,
/// whether or not the pointer rests on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Countdown {
    pub until: chrono::DateTime<chrono::Utc>,
    /// Full length in ms, for the bar's starting width.
    pub total_ms: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    pub undo: Option<UndoAction>,
    pub error: bool,
    /// Custom button: (label, action).
    pub action: Option<(String, ToastAction)>,
    pub countdown: Option<Countdown>,
}

impl Toast {
    fn new(text: String) -> Self {
        Self { id: 0, text, undo: None, error: false, action: None, countdown: None }
    }
}

/// Add a toast: Outlook shows one undo bar at a time, and at most three
/// toasts stay (the oldest go first).
fn push_bounded(toasts: &mut Vec<Toast>, toast: Toast) {
    if toast.undo.is_some() {
        toasts.retain(|t| t.undo.is_none());
    }
    toasts.push(toast);
    let excess = toasts.len().saturating_sub(3);
    toasts.drain(..excess);
}

/// A request to open the compose form (consumed by the Mail module).
#[derive(Clone, Debug, PartialEq)]
pub struct ComposeRequest {
    pub kind: ComposeKind,
    /// Message being replied to / forwarded.
    pub source: Option<MessageId>,
    /// Existing draft to continue editing.
    pub draft: Option<MessageId>,
    /// Pre-filled recipients (e.g. "Email" from a contact card).
    pub to: Vec<EmailAddress>,
    pub subject: Option<String>,
    /// Open in a separate window-like overlay instead of the reading pane.
    pub popout: bool,
}

impl ComposeRequest {
    pub fn new_message() -> Self {
        Self { kind: ComposeKind::New, source: None, draft: None, to: Vec::new(), subject: None, popout: false }
    }

    pub fn reply(kind: ComposeKind, source: MessageId) -> Self {
        Self { kind, source: Some(source), ..Self::new_message() }
    }

    pub fn to(addrs: Vec<EmailAddress>) -> Self {
        Self { to: addrs, ..Self::new_message() }
    }
}

/// Cross-module navigation (e.g. open the email behind a flagged task).
#[derive(Clone, Debug, PartialEq)]
pub enum NavRequest {
    Folder(FolderId),
    Message(MessageId),
    Event(EventId),
    /// Open the calendar on a date (and optionally start a new event there).
    CalendarDate(chrono::NaiveDate),
    NewEvent { start: chrono::DateTime<chrono::Utc>, end: chrono::DateTime<chrono::Utc>, all_day: bool },
    Contact(ContactId),
    TaskList(TaskListId),
    SmartList(SmartList),
    /// People's New contact form, pre-filled ("Add to contacts" from Mail).
    NewContact { name: String, email: String },
    /// Calendar's new-event form with attendees ("Schedule meeting" from People).
    NewEventWith { start: chrono::DateTime<chrono::Utc>, end: chrono::DateTime<chrono::Utc>, attendees: Vec<EmailAddress> },
    /// A task: To Do opens its list, selects it and shows the detail pane.
    Task(TaskId),
}

impl NavRequest {
    /// The module that consumes this request.
    pub fn module(&self) -> Module {
        match self {
            NavRequest::Folder(_) | NavRequest::Message(_) => Module::Mail,
            NavRequest::Event(_) | NavRequest::CalendarDate(_) | NavRequest::NewEvent { .. } | NavRequest::NewEventWith { .. } => Module::Calendar,
            NavRequest::Contact(_) | NavRequest::NewContact { .. } => Module::People,
            NavRequest::TaskList(_) | NavRequest::SmartList(_) | NavRequest::Task(_) => Module::Tasks,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchState {
    /// Text in the header search box.
    pub query: String,
    /// Search submitted (Enter / suggestion picked). Results replace the list.
    pub active: bool,
    /// Bumped on every submit so modules can react to repeated searches.
    pub serial: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsPage {
    #[default]
    Accounts,
    General,
    Mail,
    Calendar,
    People,
}

#[derive(Clone, Copy)]
pub struct AppState {
    pub module: Signal<Module>,
    pub settings: Signal<Settings>,
    pub mail_rev: Signal<u64>,
    pub calendar_rev: Signal<u64>,
    pub people_rev: Signal<u64>,
    pub tasks_rev: Signal<u64>,
    pub accounts_rev: Signal<u64>,
    pub toasts: Signal<Vec<Toast>>,
    pub compose: Signal<Option<ComposeRequest>>,
    pub nav: Signal<Option<NavRequest>>,
    pub search: Signal<SearchState>,
    pub settings_dialog: Signal<Option<SettingsPage>>,
    pub my_day_open: Signal<bool>,
    /// Ribbon commands for the open compose form: (serial, command id).
    /// Sent by the Mail ribbon via [`AppState::compose_command`], consumed by
    /// `mail::compose`.
    pub compose_cmd: Signal<Option<(u64, String)>>,
    next_toast: Signal<u64>,
}

pub fn use_app() -> AppState {
    use_context::<AppState>()
}

/// Create the state and provide it to the tree. Call once, in the root.
pub fn provide_app_state() -> AppState {
    let settings = store().settings();
    use_context_provider(|| AppState {
        module: Signal::new(Module::Mail),
        settings: Signal::new(settings),
        mail_rev: Signal::new(0),
        calendar_rev: Signal::new(0),
        people_rev: Signal::new(0),
        tasks_rev: Signal::new(0),
        accounts_rev: Signal::new(0),
        toasts: Signal::new(Vec::new()),
        compose: Signal::new(None),
        nav: Signal::new(None),
        search: Signal::new(SearchState::default()),
        settings_dialog: Signal::new(None),
        my_day_open: Signal::new(false),
        compose_cmd: Signal::new(None),
        next_toast: Signal::new(1),
    })
}

impl AppState {
    // Subscriptions: call at the top of a component before querying the store.
    pub fn mail_rev(&self) -> u64 { *self.mail_rev.read() }
    pub fn calendar_rev(&self) -> u64 { *self.calendar_rev.read() }
    pub fn people_rev(&self) -> u64 { *self.people_rev.read() }
    pub fn tasks_rev(&self) -> u64 { *self.tasks_rev.read() }
    pub fn accounts_rev(&self) -> u64 { *self.accounts_rev.read() }

    pub fn bump_mail(mut self) { self.mail_rev += 1; }
    pub fn bump_calendar(mut self) { self.calendar_rev += 1; }
    pub fn bump_people(mut self) { self.people_rev += 1; }
    pub fn bump_tasks(mut self) { self.tasks_rev += 1; }
    pub fn bump_accounts(mut self) {
        self.accounts_rev += 1;
        self.mail_rev += 1;
        self.calendar_rev += 1;
        self.people_rev += 1;
        self.tasks_rev += 1;
    }

    /// Change settings, persist them and re-render dependents.
    pub fn update_settings(mut self, f: impl FnOnce(&mut Settings)) {
        let mut s = self.settings.read().clone();
        f(&mut s);
        if let Err(e) = store().save_settings(&s) {
            self.error(format!("Couldn't save settings: {e}"));
        }
        self.settings.set(s);
    }

    pub fn go(mut self, module: Module) {
        self.module.set(module);
    }

    pub fn navigate(mut self, req: NavRequest) {
        self.module.set(req.module());
        self.nav.set(Some(req));
    }

    /// Open compose (switches to Mail).
    pub fn compose(mut self, req: ComposeRequest) {
        self.module.set(Module::Mail);
        self.compose.set(Some(req));
    }

    /// Forward a ribbon command to the open compose form.
    pub fn compose_command(mut self, id: impl Into<String>) {
        let serial = self.compose_cmd.read().as_ref().map(|(n, _)| n + 1).unwrap_or(1);
        self.compose_cmd.set(Some((serial, id.into())));
    }

    pub fn toast(self, text: impl Into<String>) -> u64 {
        self.push_toast(Toast::new(text.into()))
    }

    pub fn toast_with_undo(self, text: impl Into<String>, undo: UndoAction) -> u64 {
        self.push_toast(Toast { undo: Some(undo), ..Toast::new(text.into()) })
    }

    /// A toast with its own button, e.g. "Exported 3 contacts" + "Open folder".
    pub fn toast_with_action(self, text: impl Into<String>, label: impl Into<String>, action: ToastAction) -> u64 {
        self.push_toast(Toast { action: Some((label.into(), action)), ..Toast::new(text.into()) })
    }

    /// The "Undo send" bar: Undo plus a countdown that ends at `until`
    /// (the `settings.undo_send_secs` delay). Undo runs `undo`.
    pub fn toast_undo_send(self, text: impl Into<String>, undo: UndoAction, until: chrono::DateTime<chrono::Utc>) -> u64 {
        let total_ms = (until - chrono::Utc::now()).num_milliseconds().max(0) as u64;
        self.push_toast(Toast { undo: Some(undo), countdown: Some(Countdown { until, total_ms }), ..Toast::new(text.into()) })
    }

    pub fn error(self, text: impl Into<String>) -> u64 {
        self.push_toast(Toast { error: true, ..Toast::new(text.into()) })
    }

    fn push_toast(mut self, mut toast: Toast) -> u64 {
        let id = *self.next_toast.read();
        self.next_toast += 1;
        toast.id = id;
        let mut toasts = self.toasts.write();
        push_bounded(&mut toasts, toast);
        id
    }

    /// Run a toast's custom button.
    pub fn run_toast_action(mut self, action: &ToastAction) {
        match action {
            ToastAction::OpenPath(path) => {
                let spawned = std::process::Command::new("xdg-open")
                    .arg(path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                if let Err(e) = spawned {
                    self.error(format!("Couldn't open {}: {e}", path.display()));
                }
            }
            ToastAction::Navigate(req) => self.navigate(req.clone()),
            ToastAction::OpenSettings(page) => self.settings_dialog.set(Some(*page)),
        }
    }

    pub fn dismiss_toast(mut self, id: u64) {
        self.toasts.write().retain(|t| t.id != id);
    }

    /// Run an undo action against the store and refresh.
    pub fn run_undo(self, action: &UndoAction) {
        let s = store();
        let result = match action {
            UndoAction::Moves(records) => s.undo_moves(records).map(|_| self.bump_mail()),
            UndoAction::RestoreTask(t) => s.insert_task(&mut Task { id: 0, ..t.clone() }).map(|_| self.bump_tasks()),
            UndoAction::RestoreEvent(e) => s.insert_event(&mut Event { id: 0, ..e.clone() }).map(|_| self.bump_calendar()),
            UndoAction::RestoreContact(c) => s.insert_contact(&mut Contact { id: 0, ..c.clone() }).map(|_| self.bump_people()),
            UndoAction::RestoreMessages(msgs) => msgs.iter().try_for_each(|m| s.update_message(m)).map(|_| self.bump_mail()),
        };
        if let Err(e) = result {
            self.error(format!("Couldn't undo: {e}"));
        }
    }

    /// Report a store error as a toast; returns the value on success.
    pub fn check<T>(self, r: Result<T>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.error(e.to_string());
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toast(id: u64, undo: bool) -> Toast {
        Toast { id, undo: undo.then(|| UndoAction::Moves(Vec::new())), ..Toast::new(format!("t{id}")) }
    }

    #[test]
    fn new_requests_go_to_their_modules() {
        let now = chrono::Utc::now();
        assert_eq!(NavRequest::NewContact { name: "Ada".into(), email: "ada@example.com".into() }.module(), Module::People);
        assert_eq!(NavRequest::NewEventWith { start: now, end: now, attendees: Vec::new() }.module(), Module::Calendar);
        assert_eq!(NavRequest::Task(7).module(), Module::Tasks);
        assert_eq!(NavRequest::Message(1).module(), Module::Mail);
        assert_eq!(NavRequest::SmartList(SmartList::MyDay).module(), Module::Tasks);
    }

    #[test]
    fn one_undo_bar_and_three_toasts_at_most() {
        let mut list = Vec::new();
        push_bounded(&mut list, toast(1, true));
        push_bounded(&mut list, toast(2, false));
        push_bounded(&mut list, toast(3, true));
        assert_eq!(list.iter().map(|t| t.id).collect::<Vec<_>>(), [2, 3], "a new undo bar replaces the old one");
        push_bounded(&mut list, toast(4, false));
        push_bounded(&mut list, toast(5, false));
        assert_eq!(list.iter().map(|t| t.id).collect::<Vec<_>>(), [3, 4, 5]);
        let with_action = Toast { action: Some(("Open folder".into(), ToastAction::OpenPath("/tmp".into()))), ..Toast::new("Exported".into()) };
        push_bounded(&mut list, with_action);
        assert_eq!(list.len(), 3);
        assert!(list.last().is_some_and(|t| t.action.as_ref().is_some_and(|(l, _)| l == "Open folder")));
    }
}
