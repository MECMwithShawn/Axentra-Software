//! What a backup holds: the JSON document inside the encrypted file.
//!
//! Every struct takes serde defaults so a backup written by a later 1.x
//! build (with extra fields) still opens here.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::*;

/// `manifest.format` of every backup.
pub const FORMAT_NAME: &str = "axentra-outlook-backup";

/// Item counts, used by the manifest, the restore preview and the restore
/// summary (added / skipped).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Counts {
    pub accounts: usize,
    pub folders: usize,
    pub messages: usize,
    pub drafts: usize,
    pub attachments: usize,
    pub calendars: usize,
    pub events: usize,
    pub contacts: usize,
    pub contact_lists: usize,
    pub task_lists: usize,
    pub tasks: usize,
    pub categories: usize,
}

impl Counts {
    pub fn add(&mut self, o: &Counts) {
        self.accounts += o.accounts;
        self.folders += o.folders;
        self.messages += o.messages;
        self.drafts += o.drafts;
        self.attachments += o.attachments;
        self.calendars += o.calendars;
        self.events += o.events;
        self.contacts += o.contacts;
        self.contact_lists += o.contact_lists;
        self.task_lists += o.task_lists;
        self.tasks += o.tasks;
        self.categories += o.categories;
    }

    /// The user-facing items with a non-zero count, e.g. `("messages", 12)`.
    /// Folders, calendars and lists are containers and left out.
    pub fn items(&self) -> Vec<(&'static str, usize)> {
        [
            ("accounts", self.accounts),
            ("messages", self.messages),
            ("drafts", self.drafts),
            ("attachments", self.attachments),
            ("events", self.events),
            ("contacts", self.contacts),
            ("contact lists", self.contact_lists),
            ("tasks", self.tasks),
            ("categories", self.categories),
        ]
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .collect()
    }

    /// Everything that is data (not containers).
    pub fn total_items(&self) -> usize {
        self.messages + self.drafts + self.events + self.contacts + self.contact_lists + self.tasks
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    /// Always [`FORMAT_NAME`].
    pub format: String,
    pub format_version: u16,
    /// Version of the app that wrote the backup.
    pub app_version: String,
    pub created_at: DateTime<Utc>,
    pub counts: Counts,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            format: FORMAT_NAME.into(),
            format_version: super::format::FORMAT_VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            counts: Counts::default(),
        }
    }
}

/// The whole backup document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Payload {
    pub manifest: Manifest,
    pub settings: Settings,
    pub categories: Vec<Category>,
    pub recent_addresses: Vec<RecentAddress>,
    pub blocked_senders: Vec<BlockedSender>,
    pub safe_senders: Vec<SafeSender>,
    pub inference_overrides: Vec<InferenceOverride>,
    /// App-wide key/value entries from the allowlist (see `collect.rs`).
    pub kv: Vec<KvEntry>,
    pub accounts: Vec<AccountData>,
}

/// One account: its setup (no secrets) and the data that exists only on
/// this device.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountData {
    /// The account as stored, `config` stripped of anything secret-looking.
    pub account: Account,
    /// Everything of the account is included (local accounts). For synced
    /// accounts only what has no copy on a server is.
    pub complete: bool,
    /// All folders (complete accounts only), parents before children.
    pub folders: Vec<Folder>,
    /// `mail_special_folders` rows: (kind, folder id), e.g. the Snoozed folder.
    pub special_folders: Vec<(String, FolderId)>,
    pub messages: Vec<MessageData>,
    pub calendars: Vec<CalendarData>,
    pub contacts: Vec<Contact>,
    pub contact_lists: Vec<ContactList>,
    pub task_lists: Vec<TaskListData>,
    pub sweep_rules: Vec<SweepRule>,
    /// Account-scoped key/value entries (ribbon layout, calendar groups).
    pub kv: Vec<AccountKv>,
}

impl AccountData {
    pub fn counts(&self) -> Counts {
        let drafts = self.messages.iter().filter(|m| m.message.is_draft).count();
        Counts {
            accounts: 1,
            folders: self.folders.len(),
            messages: self.messages.len() - drafts,
            drafts,
            attachments: self.messages.iter().map(|m| m.attachments.len()).sum(),
            calendars: self.calendars.len(),
            events: self.calendars.iter().map(|c| c.events.len()).sum(),
            contacts: self.contacts.len(),
            contact_lists: self.contact_lists.len(),
            task_lists: self.task_lists.len(),
            tasks: self.task_lists.iter().map(|l| l.tasks.len()).sum(),
            categories: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MessageData {
    pub message: Message,
    pub body: Option<BodyData>,
    pub attachments: Vec<AttachmentData>,
    /// The editable draft (`drafts` table) behind a draft message.
    pub draft: Option<Draft>,
    /// Folder the message was deleted from (`mail_origins`).
    pub origin_folder: Option<FolderId>,
    /// Message-ID of the message a reply/forward draft answers, to relink
    /// it when that message isn't part of the backup (synced mail).
    pub draft_source_message_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BodyData {
    pub html: Option<String>,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AttachmentData {
    pub attachment: Attachment,
    /// The file's bytes when they were stored locally.
    #[serde(with = "b64")]
    pub data: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarData {
    pub calendar: Calendar,
    /// Only a container for local-only events: the calendar itself lives on
    /// the server.
    pub container_only: bool,
    pub events: Vec<Event>,
    pub reminder_states: Vec<ReminderState>,
}

/// A dismissed or snoozed reminder (`reminder_state`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReminderState {
    pub event_id: EventId,
    pub occurrence_start: i64,
    pub dismissed: bool,
    pub snoozed_until: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskListData {
    pub list: TaskList,
    /// Only a container for local-only tasks (see [`CalendarData::container_only`]).
    pub container_only: bool,
    pub tasks: Vec<Task>,
}

/// A folder named by role, or by its path of names for custom folders, so
/// rules can be pointed at the right folder of another store.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderRef {
    pub role: FolderRole,
    pub path: Vec<String>,
}

/// A saved Sweep rule (`mail_sweep_rules`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SweepRule {
    pub folder: FolderRef,
    pub sender: String,
    pub mode: String,
    pub target: FolderRef,
    pub created_at: i64,
    pub last_run: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentAddress {
    pub address: String,
    pub name: String,
    pub last_used: i64,
    pub use_count: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BlockedSender {
    pub address: String,
    pub name: String,
    pub added_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SafeSender {
    pub address: String,
    pub added_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InferenceOverride {
    pub address: String,
    pub inference: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KvEntry {
    pub key: String,
    pub value: String,
}

/// An account-scoped key `<prefix><account id><rest>`, stored without the
/// id so it can be written back under the restored account's id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountKv {
    pub prefix: String,
    pub rest: String,
    pub value: String,
}

/// Optional bytes as standard base64 (attachments inside the JSON).
mod b64 {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(bytes) => s.serialize_some(&STANDARD.encode(bytes)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let text: Option<String> = Option::deserialize(d)?;
        text.map(|t| STANDARD.decode(t.as_bytes()).map_err(D::Error::custom)).transpose()
    }
}
