//! Domain model shared by storage, sync and UI.
//!
//! Field names and enums are part of the frozen contract described in
//! `docs/ARCHITECTURE.md`. Add fields only with a `#[serde(default)]` so
//! older rows keep deserialising.

use chrono::{DateTime, NaiveDate, Utc, Weekday};
use serde::{Deserialize, Serialize};

pub type AccountId = i64;
pub type FolderId = i64;
pub type MessageId = i64;
pub type AttachmentId = i64;
pub type CalendarId = i64;
pub type EventId = i64;
pub type ContactId = i64;
pub type ContactListId = i64;
pub type TaskListId = i64;
pub type TaskId = i64;

// ---------------------------------------------------------------- people bits

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EmailAddress {
    pub name: String,
    pub address: String,
}

impl EmailAddress {
    pub fn new(name: impl Into<String>, address: impl Into<String>) -> Self {
        Self { name: name.into(), address: address.into() }
    }

    /// Name if present, otherwise the address.
    pub fn display(&self) -> &str {
        if self.name.trim().is_empty() { &self.address } else { &self.name }
    }

    /// Up to two initials, e.g. "Megan Bowen" -> "MB", "info@x.com" -> "I".
    pub fn initials(&self) -> String {
        crate::text::initials(self.display())
    }
}

// ---------------------------------------------------------------- accounts

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccountKind {
    /// Offline account that lives only in the local database (used for the demo data).
    #[default]
    Local,
    /// Generic IMAP + SMTP account.
    Imap,
    /// Microsoft 365 / Outlook.com via Microsoft Graph.
    Microsoft,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub kind: AccountKind,
    pub display_name: String,
    pub email: String,
    /// CSS colour used for the account badge.
    pub color: String,
    /// Provider-specific connection settings, interpreted by `axentra-sync`.
    /// Never contains secrets: passwords and tokens live in the OS keyring.
    pub config: serde_json::Value,
    pub sort_order: i64,
    pub created_at: DateTime<Utc>,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub sync_error: Option<String>,
    /// Optional short name shown instead of the address in the folder pane
    /// and account manager (Settings > Accounts > Manage > Nickname).
    #[serde(default)]
    pub nickname: String,
}

/// The signed-in user's own profile photo for an account, as the sync
/// engine last fetched it (Microsoft Graph `/me/photo`). Kept per account by
/// the store (`account_photos`), removed with the account, never backed up.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountPhoto {
    /// `@odata.mediaEtag` of the stored photo ("" when there is none).
    #[serde(default)]
    pub etag: String,
    /// `data:image/<type>;base64,…`, or `None` when the account has no photo.
    #[serde(default)]
    pub data_url: Option<String>,
    /// When the server was last asked successfully.
    #[serde(default)]
    pub checked_at: Option<DateTime<Utc>>,
    /// When the last attempt failed (network, throttling, …); retried
    /// sooner than a regular re-check.
    #[serde(default)]
    pub failed_at: Option<DateTime<Utc>>,
    /// `@odata.mediaEtag` of a photo that was downloaded and not kept (too
    /// large, not an image): it isn't downloaded again while the server
    /// still has it.
    #[serde(default)]
    pub rejected_etag: String,
}

// ---------------------------------------------------------------- folders

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FolderRole {
    Inbox,
    Drafts,
    Sent,
    Deleted,
    Junk,
    Archive,
    Outbox,
    Notes,
    ConversationHistory,
    #[default]
    Custom,
}

impl FolderRole {
    pub const ALL: [FolderRole; 10] = [
        FolderRole::Inbox,
        FolderRole::Drafts,
        FolderRole::Sent,
        FolderRole::Deleted,
        FolderRole::Junk,
        FolderRole::Archive,
        FolderRole::Outbox,
        FolderRole::Notes,
        FolderRole::ConversationHistory,
        FolderRole::Custom,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            FolderRole::Inbox => "inbox",
            FolderRole::Drafts => "drafts",
            FolderRole::Sent => "sent",
            FolderRole::Deleted => "deleted",
            FolderRole::Junk => "junk",
            FolderRole::Archive => "archive",
            FolderRole::Outbox => "outbox",
            FolderRole::Notes => "notes",
            FolderRole::ConversationHistory => "conversation_history",
            FolderRole::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> FolderRole {
        Self::ALL.into_iter().find(|r| r.as_str() == s).unwrap_or(FolderRole::Custom)
    }

    /// Default display name, as Outlook names the well-known folders.
    pub fn default_name(self) -> &'static str {
        match self {
            FolderRole::Inbox => "Inbox",
            FolderRole::Drafts => "Drafts",
            FolderRole::Sent => "Sent Items",
            FolderRole::Deleted => "Deleted Items",
            FolderRole::Junk => "Junk Email",
            FolderRole::Archive => "Archive",
            FolderRole::Outbox => "Outbox",
            FolderRole::Notes => "Notes",
            FolderRole::ConversationHistory => "Conversation History",
            FolderRole::Custom => "Folder",
        }
    }

    /// Fluent icon name used in the folder pane.
    pub fn icon(self) -> &'static str {
        match self {
            FolderRole::Inbox => "mail_inbox",
            FolderRole::Drafts => "drafts",
            FolderRole::Sent => "send",
            FolderRole::Deleted => "delete",
            FolderRole::Junk => "mail_prohibited",
            FolderRole::Archive => "archive",
            FolderRole::Outbox => "mail_warning",
            FolderRole::Notes => "note",
            FolderRole::ConversationHistory => "chat",
            FolderRole::Custom => "folder",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    pub account_id: AccountId,
    pub parent_id: Option<FolderId>,
    pub name: String,
    pub role: FolderRole,
    pub remote_id: Option<String>,
    pub sort_order: i64,
    pub is_favorite: bool,
    pub is_expanded: bool,
    /// Computed when loaded: unread messages in this folder.
    pub unread_count: i64,
    /// Computed when loaded: all messages in this folder.
    pub total_count: i64,
}

// ---------------------------------------------------------------- mail

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Importance {
    Low,
    #[default]
    Normal,
    High,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FlagStatus {
    #[default]
    NotFlagged,
    Flagged,
    Complete,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InferenceClass {
    #[default]
    Focused,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MeetingMessageKind {
    Request,
    Cancellation,
    Accepted,
    Tentative,
    Declined,
}

/// A message as shown in lists and the reading-pane header. Bodies are loaded
/// separately with [`crate::Store::message_body`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub account_id: AccountId,
    pub folder_id: FolderId,
    /// Messages sharing this id form one conversation.
    pub conversation_id: String,
    pub remote_id: Option<String>,
    /// RFC 5322 Message-ID header, without angle brackets.
    pub internet_message_id: String,
    pub in_reply_to: Option<String>,
    pub from: EmailAddress,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    pub subject: String,
    /// Plain-text preview, at most ~200 chars, whitespace collapsed.
    pub preview: String,
    pub received_at: DateTime<Utc>,
    pub sent_at: Option<DateTime<Utc>>,
    pub is_read: bool,
    pub flag: FlagStatus,
    pub flag_due: Option<NaiveDate>,
    pub is_pinned: bool,
    pub importance: Importance,
    pub has_attachments: bool,
    pub categories: Vec<String>,
    pub inference: InferenceClass,
    pub is_draft: bool,
    /// True when the user is @mentioned in the body.
    pub mentions_me: bool,
    pub meeting_kind: Option<MeetingMessageKind>,
    pub meeting_event_id: Option<EventId>,
    /// Hidden until this time (snooze).
    pub snoozed_until: Option<DateTime<Utc>>,
    /// Drafts only: when a scheduled send will go out (list shows
    /// "Scheduled: {date time}"). Mirrors [`Draft::scheduled_send_at`].
    #[serde(default)]
    pub scheduled_send_at: Option<DateTime<Utc>>,
    /// `References` header (RFC 5322): Message-IDs of the thread, oldest
    /// first. Replies get the original's list plus its own Message-ID.
    #[serde(default)]
    pub references: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: AttachmentId,
    pub message_id: MessageId,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MessageBody {
    pub message_id: MessageId,
    /// Raw HTML as received. Always pass through [`crate::html::sanitize_email_html`]
    /// before rendering.
    pub html: Option<String>,
    pub text: String,
    pub attachments: Vec<Attachment>,
}

/// Which messages a list shows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MessageQuery {
    pub folder_id: Option<FolderId>,
    /// Only used for Inbox when Focused Inbox is on.
    pub inference: Option<InferenceClass>,
    pub filter: MessageFilter,
    pub sort: MessageSort,
    /// Free-text / keyword search (see `docs/spec/mail.md`), searched across
    /// all folders of all accounts when `folder_id` is `None`.
    pub search: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MessageFilter {
    #[default]
    All,
    Unread,
    Flagged,
    ToMe,
    HasFiles,
    Mentions,
    Pinned,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MessageSort {
    #[default]
    DateNewest,
    DateOldest,
    From,
    Subject,
    Size,
    Importance,
    Flag,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ComposeKind {
    #[default]
    New,
    Reply,
    ReplyAll,
    Forward,
}

/// A draft being edited. Persisted as a message in the Drafts folder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    /// `None` until first saved.
    pub id: Option<MessageId>,
    pub account_id: AccountId,
    pub kind: ComposeKind,
    /// Message being replied to / forwarded.
    pub source_message_id: Option<MessageId>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub subject: String,
    pub body_html: String,
    pub importance: Importance,
    pub request_read_receipt: bool,
    pub attachments: Vec<DraftAttachment>,
    pub scheduled_send_at: Option<DateTime<Utc>>,
    /// Ask for a delivery receipt (SMTP DSN / Graph `isDeliveryReceiptRequested`).
    #[serde(default)]
    pub request_delivery_receipt: bool,
    /// Set while the message waits out the "Undo send" delay in Outbox. A
    /// held message is still `is_draft`; sync must not submit it yet.
    #[serde(default)]
    pub send_pending_until: Option<DateTime<Utc>>,
    /// Compose in plain text instead of HTML.
    #[serde(default)]
    pub plain_text: bool,
    /// When the draft was first saved; set by `Store::save_draft` and kept
    /// across edits (the message's `received_at` is the last save). Backups
    /// use it to recognise a draft that was edited since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DraftAttachment {
    pub name: String,
    pub content_type: String,
    pub size: i64,
    /// Local file path the attachment was picked from.
    pub path: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    /// One of the preset colour names, see `CATEGORY_COLORS`.
    pub color: String,
}

/// Preset category colours: (name, light-theme hex, dark-theme hex).
pub const CATEGORY_COLORS: &[(&str, &str, &str)] = &[
    ("red", "#e74856", "#ff6b78"),
    ("orange", "#ff8c00", "#ffa53d"),
    ("peach", "#f7a35c", "#f7b47f"),
    ("yellow", "#fde300", "#ffe94d"),
    ("green", "#13a10e", "#3fc23a"),
    ("teal", "#00b294", "#3dd6b9"),
    ("olive", "#8e8c1b", "#b4b23f"),
    ("blue", "#0078d4", "#3a9bea"),
    ("purple", "#8764b8", "#a98ad6"),
    ("cranberry", "#c50f1f", "#e34a58"),
    ("steel", "#486991", "#6f8fb5"),
    ("darksteel", "#394146", "#5f6a71"),
    ("gray", "#7a7574", "#a19f9d"),
    ("darkgray", "#525050", "#7b7978"),
    ("black", "#000000", "#cccccc"),
    ("darkred", "#750b1c", "#a83a4b"),
    ("darkorange", "#ca5010", "#e3782f"),
    ("brown", "#8e562e", "#b07a52"),
    ("gold", "#c19c00", "#dbbc3a"),
    ("darkgreen", "#0b6a0b", "#2f962f"),
    ("darkteal", "#005b70", "#2d8aa0"),
    ("darkolive", "#5c5c18", "#88882f"),
    ("darkblue", "#004e8c", "#2f7bba"),
    ("darkpurple", "#5c2e91", "#8758bd"),
    ("darkcranberry", "#a4262c", "#c75157"),
];

// ---------------------------------------------------------------- calendar

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Calendar {
    pub id: CalendarId,
    pub account_id: AccountId,
    pub name: String,
    /// CSS colour.
    pub color: String,
    pub is_visible: bool,
    pub is_default: bool,
    pub can_edit: bool,
    pub remote_id: Option<String>,
    /// Calendar group heading in the left pane, e.g. "My calendars".
    pub group: String,
    pub sort_order: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShowAs {
    Free,
    WorkingElsewhere,
    Tentative,
    #[default]
    Busy,
    OutOfOffice,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResponseStatus {
    #[default]
    None,
    Organizer,
    Accepted,
    Tentative,
    Declined,
    NotResponded,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AttendeeKind {
    #[default]
    Required,
    Optional,
    Resource,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Attendee {
    pub email: EmailAddress,
    pub kind: AttendeeKind,
    pub response: ResponseStatus,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecurrenceFreq {
    Daily,
    #[default]
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecurrenceEnd {
    #[default]
    Never,
    Until(NaiveDate),
    Count(u32),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Recurrence {
    pub freq: RecurrenceFreq,
    /// Every N days/weeks/months/years (>= 1).
    pub interval: u32,
    /// Weekly: which weekdays. Empty means the start date's weekday.
    pub by_weekday: Vec<Weekday>,
    /// Monthly/yearly "on day N".
    pub month_day: Option<u32>,
    /// Monthly/yearly "on the Nth <weekday>": (1..=4, or -1 for last, weekday).
    pub nth_weekday: Option<(i32, Weekday)>,
    /// Yearly: month 1..=12.
    pub month: Option<u32>,
    pub end: RecurrenceEnd,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    pub calendar_id: CalendarId,
    pub remote_id: Option<String>,
    pub title: String,
    pub location: String,
    pub body_html: String,
    /// For all-day events: local midnight of the first day, as UTC.
    pub start: DateTime<Utc>,
    /// Exclusive end. For all-day events: local midnight after the last day.
    pub end: DateTime<Utc>,
    pub all_day: bool,
    /// IANA zone name the event was created in, e.g. "Europe/London".
    pub time_zone: String,
    pub recurrence: Option<Recurrence>,
    /// Original start dates (local) of deleted occurrences.
    pub exdates: Vec<NaiveDate>,
    pub show_as: ShowAs,
    pub is_private: bool,
    pub reminder_minutes: Option<i32>,
    pub organizer: Option<EmailAddress>,
    pub attendees: Vec<Attendee>,
    pub my_response: ResponseStatus,
    pub is_online_meeting: bool,
    pub online_meeting_url: Option<String>,
    pub categories: Vec<String>,
    pub is_cancelled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Set on an edited occurrence ("exception") saved as its own event: the
    /// series it was split from. The series lists the occurrence's original
    /// date in `exdates`.
    #[serde(default)]
    pub series_id: Option<EventId>,
    /// For an exception: the start of the occurrence it replaces.
    #[serde(default)]
    pub original_start: Option<DateTime<Utc>>,
    /// How the last RSVP goes to the organizer: `true` = "Respond without
    /// email" (the server records it but sends nothing).
    #[serde(default)]
    pub response_silent: bool,
    /// Note sent with the last RSVP ("Respond with note"); empty = none.
    #[serde(default)]
    pub response_note: String,
}

/// One concrete occurrence of an event inside a queried range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Occurrence {
    pub event: Event,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Start of this occurrence before any edits (used to address it).
    pub original_start: DateTime<Utc>,
    /// Calendar colour, copied for rendering convenience.
    pub color: String,
}

// ---------------------------------------------------------------- people

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LabeledValue {
    /// e.g. "Work", "Home", "Mobile", "Personal".
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PostalAddress {
    pub label: String,
    pub street: String,
    pub city: String,
    pub state: String,
    pub postal_code: String,
    pub country: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Contact {
    pub id: ContactId,
    pub account_id: AccountId,
    pub remote_id: Option<String>,
    pub given_name: String,
    pub surname: String,
    /// Usually "Given Surname"; kept separately so users can override it.
    pub display_name: String,
    pub nickname: String,
    pub emails: Vec<LabeledValue>,
    pub phones: Vec<LabeledValue>,
    pub addresses: Vec<PostalAddress>,
    pub websites: Vec<String>,
    pub company: String,
    pub job_title: String,
    pub department: String,
    pub office_location: String,
    pub birthday: Option<NaiveDate>,
    pub notes: String,
    pub is_favorite: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // Fields below were added for the People module (see docs/spec/people-todo.md 2.8).
    #[serde(default)]
    pub middle_name: String,
    /// IM addresses as URIs or handles, e.g. "xmpp:ada@example.com".
    #[serde(default)]
    pub im_addresses: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    /// Contact folder name; empty means the default "Contacts" folder.
    #[serde(default)]
    pub folder: String,
    /// Photo as a `data:image/...;base64,` URL.
    #[serde(default)]
    pub photo: Option<String>,
    /// Position in the Favorites view once the user reorders it.
    #[serde(default)]
    pub favorite_order: Option<i64>,
    /// The form's "More fields..." (boxed: they are rarely set, and it keeps
    /// `Contact` small enough to sit inline in `UndoAction`).
    #[serde(default)]
    pub more: Box<ContactMore>,
}

/// Less common contact fields (docs/spec/people-todo.md 2.8, "More fields...").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContactMore {
    /// Name prefix, e.g. "Dr." (Graph `title`).
    pub title: String,
    /// Name suffix, e.g. "Jr." (Graph `generation`).
    pub suffix: String,
    pub phonetic_given_name: String,
    pub phonetic_surname: String,
    pub phonetic_company: String,
    pub manager: String,
    pub assistant: String,
    pub profession: String,
    pub spouse: String,
    pub children: String,
    pub file_as: String,
}

impl Contact {
    pub fn primary_email(&self) -> Option<&str> {
        self.emails.first().map(|e| e.value.as_str())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ContactList {
    pub id: ContactListId,
    pub account_id: AccountId,
    pub name: String,
    pub members: Vec<EmailAddress>,
    // Fields below were added for the People module (see docs/spec/people-todo.md 2.9).
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub favorite_order: Option<i64>,
    #[serde(default)]
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------- tasks

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmartList {
    MyDay,
    Important,
    Planned,
    Assigned,
    Flagged,
    Tasks,
}

impl SmartList {
    pub const ALL: [SmartList; 6] = [
        SmartList::MyDay,
        SmartList::Important,
        SmartList::Planned,
        SmartList::Assigned,
        SmartList::Flagged,
        SmartList::Tasks,
    ];
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskList {
    pub id: TaskListId,
    pub account_id: AccountId,
    pub remote_id: Option<String>,
    pub name: String,
    /// Optional emoji shown instead of the list icon.
    pub emoji: Option<String>,
    /// The built-in "Tasks" list that new tasks land in by default.
    pub is_default: bool,
    /// Optional group heading the list sits under.
    pub group: Option<String>,
    pub sort_order: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskStep {
    pub id: i64,
    pub title: String,
    pub is_completed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub list_id: TaskListId,
    pub remote_id: Option<String>,
    pub title: String,
    pub notes: String,
    pub is_completed: bool,
    pub completed_at: Option<DateTime<Utc>>,
    pub is_important: bool,
    pub due: Option<NaiveDate>,
    pub reminder: Option<DateTime<Utc>>,
    pub recurrence: Option<Recurrence>,
    /// The day this task was added to My Day (My Day resets daily).
    pub my_day: Option<NaiveDate>,
    pub steps: Vec<TaskStep>,
    pub categories: Vec<String>,
    pub assigned_to_me: bool,
    pub linked_message_id: Option<MessageId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub sort_order: i64,
    /// Day on which "Remove from My Day" hid a task that My Day showed on its
    /// own because it is due that day.
    #[serde(default)]
    pub my_day_hidden: Option<NaiveDate>,
    /// When the task was last added to My Day (the "Added to My Day" sort).
    #[serde(default)]
    pub my_day_at: Option<DateTime<Utc>>,
    /// When the note last changed ("Updated …" under the note).
    #[serde(default)]
    pub notes_updated_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------- settings

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Density {
    Roomy,
    #[default]
    Cozy,
    Compact,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReadingPanePosition {
    #[default]
    Right,
    Bottom,
    Hidden,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConversationOrder {
    #[default]
    NewestOnTop,
    NewestOnBottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarkAsRead {
    #[default]
    OnSelect,
    AfterDelay,
    Never,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CalendarView {
    Day,
    #[default]
    WorkWeek,
    Week,
    Month,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyboardMode {
    /// New Outlook / Outlook on the web shortcuts.
    #[default]
    Outlook,
    /// Classic Outlook for Windows shortcuts.
    Classic,
    Gmail,
    Yahoo,
    Off,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeMode,
    /// Brand colour override (CSS hex), `None` = default blue.
    pub accent: Option<String>,
    pub density: Density,
    pub reading_pane: ReadingPanePosition,
    pub conversation_view: bool,
    pub conversation_order: ConversationOrder,
    pub focused_inbox: bool,
    pub show_preview_text: bool,
    pub show_sender_images: bool,
    pub show_attachment_previews: bool,
    pub mark_as_read: MarkAsRead,
    pub mark_as_read_delay_secs: u32,
    pub block_external_images: bool,
    pub empty_deleted_on_exit: bool,
    pub undo_send_secs: u32,
    pub signature_html: String,
    pub signature_on_new: bool,
    pub signature_on_reply: bool,
    pub default_font: String,
    pub default_font_size_pt: u32,
    pub use_24h_time: bool,
    pub week_start: Weekday,
    pub work_days: Vec<Weekday>,
    pub work_start_hour: u32,
    pub work_end_hour: u32,
    pub calendar_view: CalendarView,
    pub show_week_numbers: bool,
    pub default_reminder_minutes: i32,
    pub default_event_minutes: i32,
    pub keyboard_mode: KeyboardMode,
    pub simplified_ribbon: bool,
    pub folder_pane_collapsed: bool,
    pub folder_pane_width: u32,
    pub message_list_width: u32,
    pub notifications: bool,
    pub notification_sound: bool,
    // ---- Added for the Settings dialog (docs/spec/shell.md §7). ----
    /// Colour theme id from Appearance: "blue" (default), "classic_blue",
    /// "neutral", "teal", "purple", "pink", "red", "orange", "green",
    /// "graphite" (or "custom" with `accent`).
    pub color_theme: String,
    /// Keep message bodies on a light background while dark mode is on.
    pub light_message_background: bool,
    /// "Classic look": flat panes instead of floating cards.
    pub classic_look: bool,
    /// High contrast: `None` follows the system, `Some(on)` overrides it.
    pub high_contrast: Option<bool>,
    /// Reduce motion: `None` follows the system, `Some(on)` overrides it.
    pub reduce_motion: Option<bool>,
    /// Show ribbon key tips when Alt is pressed and released.
    pub show_key_tips: bool,
    /// Announce new messages through the screen-reader live region.
    pub announce_new_messages: bool,
    /// Message list format: subject on the first line instead of the sender.
    pub subject_first: bool,
    /// Preview text length in the message list: 2, 1 or 0 extra lines.
    pub preview_lines: u8,
    /// With `conversation_view`: group by branches within conversations.
    pub conversation_branches: bool,
    /// Reading pane shows only the selected message, not the whole conversation.
    pub conversation_selected_only: bool,
    /// Show deleted items inside conversations.
    pub conversation_show_deleted: bool,
    /// What the reading pane does after the open item is moved or deleted.
    pub after_move: AfterMoveAction,
    /// Folder pane: sort folders A to Z instead of the custom order.
    pub folders_sorted_az: bool,
    /// Folder pane: show the Favorites section.
    pub show_favorites: bool,
    /// Notify about new mail / event reminders.
    pub notify_mail: bool,
    pub notify_calendar: bool,
    /// Only notify for Focused Inbox mail.
    pub notify_focused_only: bool,
    /// Combine several new-mail notifications into one summary.
    pub notify_batch: bool,
    /// Default search scope.
    pub search_scope: SearchScope,
    /// Include Deleted Items in search results.
    pub search_include_deleted: bool,
    /// Offline: keep mail, calendar and people on this device.
    pub offline_enabled: bool,
    pub offline_folders: OfflineFolders,
    /// Days of email to keep offline (7, 30, 90, 180, 365, 730).
    pub offline_days: u32,
    /// Keep syncing when the window is closed.
    pub background_sync: bool,
    /// People: sort contacts by last name instead of first name.
    pub contacts_sort_by_last: bool,
    /// People: display names as "Last, First".
    pub contacts_last_first: bool,
    /// App rail modules the user unpinned ("people", "tasks").
    pub unpinned_apps: Vec<String>,
    /// Order of the movable app-rail modules (Mail and Calendar stay first).
    pub rail_order: Vec<String>,
    /// Mail > Attachments: warn before sending a message whose text mentions
    /// an attachment but has none.
    pub attachment_reminder: bool,
    // ---- Added for Settings > Accounts > Backup. ----
    /// Back up automatically every month (runs only once a backup password
    /// is remembered in the keyring).
    pub backup_auto: bool,
    /// Folder chosen for backups; `None` = the OneDrive folder (or
    /// Documents). Backups go into `Axentra/Outlook Backups` inside it.
    pub backup_dir: Option<String>,
    /// How many backups to keep; older ones are deleted.
    pub backup_keep: u32,
    // ---- Added for client-side decorations (docs/spec/shell.md §12.4). ----
    /// Use the desktop's own title bar instead of the header (read at startup).
    pub system_title_bar: bool,
}

impl Default for Settings {
    fn default() -> Self {
        use Weekday::*;
        Self {
            theme: ThemeMode::System,
            accent: None,
            density: Density::Cozy,
            reading_pane: ReadingPanePosition::Right,
            conversation_view: true,
            conversation_order: ConversationOrder::NewestOnTop,
            focused_inbox: true,
            show_preview_text: true,
            // Cozy (the default density) shows neither, like Outlook; Roomy turns them on.
            show_sender_images: false,
            show_attachment_previews: false,
            mark_as_read: MarkAsRead::OnSelect,
            mark_as_read_delay_secs: 5,
            block_external_images: true,
            empty_deleted_on_exit: false,
            undo_send_secs: 5,
            signature_html: String::new(),
            signature_on_new: true,
            signature_on_reply: true,
            default_font: "Aptos".into(),
            default_font_size_pt: 12,
            use_24h_time: false,
            week_start: Sun,
            work_days: vec![Mon, Tue, Wed, Thu, Fri],
            work_start_hour: 8,
            work_end_hour: 17,
            calendar_view: CalendarView::WorkWeek,
            show_week_numbers: false,
            default_reminder_minutes: 15,
            default_event_minutes: 30,
            keyboard_mode: KeyboardMode::Outlook,
            simplified_ribbon: true,
            folder_pane_collapsed: false,
            folder_pane_width: 240,
            message_list_width: 400,
            notifications: true,
            notification_sound: true,
            color_theme: "blue".into(),
            light_message_background: false,
            classic_look: false,
            high_contrast: None,
            reduce_motion: None,
            show_key_tips: true,
            announce_new_messages: true,
            subject_first: false,
            preview_lines: 1,
            conversation_branches: false,
            conversation_selected_only: false,
            conversation_show_deleted: false,
            after_move: AfterMoveAction::OpenNext,
            folders_sorted_az: false,
            show_favorites: true,
            notify_mail: true,
            notify_calendar: true,
            notify_focused_only: false,
            notify_batch: true,
            search_scope: SearchScope::AllFolders,
            search_include_deleted: false,
            offline_enabled: true,
            offline_folders: OfflineFolders::DefaultAndFavorites,
            offline_days: 30,
            background_sync: true,
            contacts_sort_by_last: false,
            contacts_last_first: false,
            unpinned_apps: Vec::new(),
            rail_order: Vec::new(),
            attachment_reminder: true,
            backup_auto: true,
            backup_dir: None,
            backup_keep: 12,
            system_title_bar: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AfterMoveAction {
    #[default]
    OpenNext,
    OpenPrevious,
    ReturnToList,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SearchScope {
    #[default]
    AllFolders,
    CurrentFolder,
    CurrentMailbox,
}

/// Which folders are kept offline (Settings > General > Offline).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OfflineFolders {
    /// Inbox, Sent, Drafts, Archive, Deleted and Snoozed.
    Default,
    #[default]
    DefaultAndFavorites,
    DefaultFavoritesAndRecent,
}
