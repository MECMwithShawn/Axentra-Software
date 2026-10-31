//! App-wide state shared through Dioxus context.
//!
//! The store itself is not reactive. Components subscribe to a domain by
//! reading its revision signal (e.g. `app.mail_rev()`) before querying the
//! store, and every mutation is followed by the matching `bump_*` call.

use std::collections::HashMap;
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
    /// Undo a Report junk / Report phishing / Not junk (or a rule change):
    /// the confirmed labels go back to what they were, any rule made with it
    /// goes, the messages move back, all in one change.
    Junk { events: Vec<i64>, rules: Vec<axentra_core::junk::RuleChange>, moves: Vec<MoveRecord> },
    /// Put back a sender rule removed in Settings.
    RestoreRule(axentra_core::junk::SenderRule),
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
    /// The user's own account photos (account id → `data:image/…` URL),
    /// fetched by the sync engine; reloaded with [`AppState::bump_accounts`].
    pub account_photos: Signal<HashMap<AccountId, String>>,
    /// The Ctrl+Z stack, newest last, at most [`UNDO_DEPTH`]: every
    /// reversible action of every module (with the undo bar it showed, if
    /// any), the held "Undo send" message among them, so Ctrl+Z always
    /// takes back the most recent one, wherever focus is.
    undo: Signal<Vec<(Option<u64>, UndoAction)>>,
    next_toast: Signal<u64>,
}

/// How many actions Ctrl+Z can take back.
const UNDO_DEPTH: usize = 20;

pub fn use_app() -> AppState {
    use_context::<AppState>()
}

/// Create the state and provide it to the tree. Call once, in the root.
pub fn provide_app_state() -> AppState {
    let settings = store().settings();
    // Read now, so the first frame already shows the photos.
    let photos = store().account_photos().unwrap_or_default();
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
        account_photos: Signal::new(photos),
        undo: Signal::new(Vec::new()),
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
        // The engine bumps for a new, changed or removed photo too.
        let photos = store().account_photos().unwrap_or_default();
        if *self.account_photos.peek() != photos {
            self.account_photos.set(photos);
        }
    }

    /// The photo for one of the user's own accounts, if it has one
    /// (subscribes the caller to photo changes).
    pub fn account_photo(&self, id: AccountId) -> Option<String> {
        self.account_photos.read().get(&id).cloned()
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

    /// An undo bar; its action also goes on the Ctrl+Z stack.
    pub fn toast_with_undo(self, text: impl Into<String>, undo: UndoAction) -> u64 {
        let id = self.push_toast(Toast { undo: Some(undo.clone()), ..Toast::new(text.into()) });
        self.stack_undo(Some(id), undo);
        id
    }

    /// Put an action on the Ctrl+Z stack without an undo bar (a flag or
    /// category change, one message marked read).
    pub fn push_undo(self, undo: UndoAction) {
        self.stack_undo(None, undo);
    }

    fn stack_undo(mut self, toast: Option<u64>, undo: UndoAction) {
        let mut stack = self.undo.write();
        stack.push((toast, undo));
        let excess = stack.len().saturating_sub(UNDO_DEPTH);
        stack.drain(..excess);
    }

    /// Take an undo bar's action off the Ctrl+Z stack: a held send that went
    /// out (or was moved elsewhere) can't be taken back any more.
    pub fn forget_undo(mut self, toast: u64) {
        self.undo.write().retain(|(t, _)| *t != Some(toast));
    }

    /// The undo bar of the most recent action on the Ctrl+Z stack, if it
    /// showed one.
    pub fn newest_undo_bar(self) -> Option<u64> {
        self.undo.peek().last().and_then(|(t, _)| *t)
    }

    /// Ctrl+Z: undo the most recent reversible action and close its undo bar.
    pub fn undo_last(mut self) {
        let last = self.undo.write().pop();
        match last {
            Some((toast, action)) => {
                self.run_undo(&action);
                if let Some(id) = toast {
                    self.dismiss_toast(id);
                }
                self.toast("Undone");
            }
            None => {
                self.toast("Nothing to undo");
            }
        }
    }

    /// An undo bar's Undo button: undo its action (once: it leaves the
    /// Ctrl+Z stack too) and close the bar.
    pub fn undo_toast(self, id: u64) {
        let undo = self.toasts.peek().iter().find(|t| t.id == id).and_then(|t| t.undo.clone());
        self.forget_undo(id);
        if let Some(undo) = undo {
            self.run_undo(&undo);
        }
        self.dismiss_toast(id);
    }

    /// A toast with its own button, e.g. "Exported 3 contacts" + "Open folder".
    pub fn toast_with_action(self, text: impl Into<String>, label: impl Into<String>, action: ToastAction) -> u64 {
        self.push_toast(Toast { action: Some((label.into(), action)), ..Toast::new(text.into()) })
    }

    /// The "Undo send" bar: Undo plus a countdown that ends at `until`
    /// (the `settings.undo_send_secs` delay). Undo runs `undo`.
    /// The send is the most recent action, so it goes on the Ctrl+Z stack
    /// too; [`AppState::forget_undo`] takes it off once the message is out.
    pub fn toast_undo_send(self, text: impl Into<String>, undo: UndoAction, until: chrono::DateTime<chrono::Utc>) -> u64 {
        let total_ms = (until - chrono::Utc::now()).num_milliseconds().max(0) as u64;
        let id = self.push_toast(Toast { undo: Some(undo.clone()), countdown: Some(Countdown { until, total_ms }), ..Toast::new(text.into()) });
        self.stack_undo(Some(id), undo);
        id
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
            UndoAction::Junk { events, rules, moves } => s
                .undo_junk_operation(&axentra_core::store::JunkOperation { events: events.clone(), rules: rules.clone(), moves: moves.clone() })
                .map(|_| self.bump_mail()),
            UndoAction::RestoreRule(rule) => s.restore_sender_rule(rule).map(|_| self.bump_mail()),
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

    /// The account photos are in the state from the first frame, and
    /// `bump_accounts` (the engine's notifier) brings a changed or removed one.
    #[test]
    fn account_photos_load_at_start_and_follow_bump_accounts() {
        use std::cell::Cell;
        thread_local! {
            static APP: Cell<Option<AppState>> = const { Cell::new(None) };
        }
        fn root() -> Element {
            let app = provide_app_state();
            APP.with(|a| a.set(Some(app)));
            rsx! {}
        }

        // The only test here that needs the global store.
        let store = STORE.get_or_init(|| Arc::new(Store::open_in_memory().unwrap())).clone();
        let mut acct = Account { kind: AccountKind::Microsoft, email: "megan@contoso.example".into(), ..Default::default() };
        let id = store.add_account(&mut acct).unwrap();
        let photo = |url: &str| AccountPhoto { data_url: Some(url.into()), ..Default::default() };
        store.set_account_photo(&acct, &photo("data:image/png;base64,AAAA")).unwrap();

        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        let app = APP.with(|a| a.get()).expect("the app state");
        dom.in_runtime(|| {
            assert_eq!(app.account_photo(id).as_deref(), Some("data:image/png;base64,AAAA"));
            store.set_account_photo(&acct, &photo("data:image/jpeg;base64,BBBB")).unwrap();
            app.bump_accounts();
            assert_eq!(app.account_photo(id).as_deref(), Some("data:image/jpeg;base64,BBBB"));
            store.set_account_photo(&acct, &AccountPhoto::default()).unwrap();
            app.bump_accounts();
            assert_eq!(app.account_photo(id), None);
            store.set_account_photo(&acct, &photo("data:image/gif;base64,CCCC")).unwrap();
            store.remove_account(id).unwrap();
            app.bump_accounts();
            assert_eq!(app.account_photo(id), None, "gone with the account");
        });
    }
}
