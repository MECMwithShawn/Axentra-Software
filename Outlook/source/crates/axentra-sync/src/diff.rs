//! Change tracking without touching other areas' code: every synced item
//! has a [`Shadow`] row (store `sync_items`) holding a fingerprint of the
//! item as it was when local and server last agreed. A sync pulls server
//! changes (merging them into items the user hasn't touched), then pushes
//! whatever still differs between the local item and its shadow.

use std::collections::{HashMap, HashSet};

use axentra_core::{Contact, Event, FlagStatus, Folder, FolderId, FolderRole, InferenceClass, Message, ResponseStatus, Task, TaskList};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const KIND_MESSAGE: &str = "message";
pub const KIND_EVENT: &str = "event";
pub const KIND_CONTACT: &str = "contact";
pub const KIND_TASK: &str = "task";
pub const KIND_TASK_LIST: &str = "task_list";
pub const KIND_CALENDAR: &str = "calendar";
/// Mail folders the user can create, rename, move and delete.
pub const KIND_FOLDER: &str = "folder";

/// Server bookkeeping for one local item.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shadow {
    pub remote_id: String,
    /// Remote folder / calendar / list the item lives in on the server.
    pub container: String,
    /// IMAP UID (`None` right after a move until the copy is matched).
    pub uid: Option<u32>,
    /// Graph `changeKey` / IMAP MODSEQ.
    pub etag: Option<String>,
    /// Fingerprint of the local item when last in sync.
    pub state: Value,
    /// Provider extras (series master id, checklist id map, exdates…).
    pub extra: Value,
}

// ---------------------------------------------------------------- messages

/// The parts of a message that sync both ways.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MessageState {
    pub is_read: bool,
    pub flag: FlagStatus,
    pub flag_due: Option<NaiveDate>,
    pub folder_id: FolderId,
    pub inference: InferenceClass,
    pub categories: Vec<String>,
}

impl MessageState {
    pub fn of(m: &Message) -> Self {
        MessageState {
            is_read: m.is_read,
            flag: m.flag,
            flag_due: m.flag_due,
            folder_id: m.folder_id,
            inference: m.inference,
            categories: m.categories.clone(),
        }
    }

    pub fn from_value(v: &Value) -> Self {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// One server operation needed to bring the server up to the local state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageOp {
    Move(FolderId),
    Read(bool),
    Flag(FlagStatus, Option<NaiveDate>),
    Inference(InferenceClass),
    Categories(Vec<String>),
    /// The local message is gone (deleted permanently).
    Delete,
}

/// What to push for one message. `local` is `None` when the user removed it.
pub fn message_ops(local: Option<&Message>, last: &MessageState) -> Vec<MessageOp> {
    let Some(m) = local else { return vec![MessageOp::Delete] };
    let mut ops = Vec::new();
    if m.folder_id != last.folder_id {
        ops.push(MessageOp::Move(m.folder_id));
    }
    if m.is_read != last.is_read {
        ops.push(MessageOp::Read(m.is_read));
    }
    if m.flag != last.flag || (m.flag == FlagStatus::Flagged && m.flag_due != last.flag_due) {
        ops.push(MessageOp::Flag(m.flag, m.flag_due));
    }
    if m.inference != last.inference {
        ops.push(MessageOp::Inference(m.inference));
    }
    if m.categories != last.categories {
        ops.push(MessageOp::Categories(m.categories.clone()));
    }
    ops
}

/// Apply a server state to a local message field by field: fields the user
/// changed since the last sync keep the local value (it is pushed later);
/// untouched fields take the server value. `last` becomes the server state.
/// Returns true when the local message changed.
pub fn merge_server_message(local: &mut Message, last: &mut MessageState, server: &MessageState) -> bool {
    let before = MessageState::of(local);
    if local.is_read == last.is_read {
        local.is_read = server.is_read;
    }
    if local.flag == last.flag && local.flag_due == last.flag_due {
        local.flag = server.flag;
        local.flag_due = server.flag_due;
    }
    if local.folder_id == last.folder_id {
        local.folder_id = server.folder_id;
    }
    if local.inference == last.inference {
        local.inference = server.inference;
    }
    if local.categories == last.categories {
        local.categories = server.categories.clone();
    }
    *last = server.clone();
    MessageState::of(local) != before
}

// ---------------------------------------------------------------- folders

/// Name and parent of a custom mail folder as last agreed with the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderState {
    pub name: String,
    pub parent: Option<FolderId>,
}

impl FolderState {
    pub fn of(f: &Folder) -> Self {
        FolderState { name: f.name.clone(), parent: f.parent_id }
    }

    pub fn from_value(v: &Value) -> Self {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// A folder change made here that the server doesn't have yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FolderOp {
    /// Created locally (no server copy yet).
    Create(FolderId),
    /// Renamed and/or moved under another parent.
    Update(FolderId),
    /// Deleted locally; `remote` is the server id / mailbox name.
    Delete { local_id: FolderId, remote: String },
}

/// Folder changes to push for one account. Creates and updates come
/// parents first; deletes come children first (longest server name first).
/// Only custom folders take part: well-known folders are the server's.
pub fn folder_ops(locals: &[Folder], shadows: &[(i64, Shadow)]) -> Vec<FolderOp> {
    let by_id: HashMap<FolderId, &Folder> = locals.iter().map(|f| (f.id, f)).collect();
    let shadow_of: HashMap<FolderId, &Shadow> = shadows.iter().map(|(id, s)| (*id, s)).collect();
    let depth = |f: &Folder| {
        let mut d = 0;
        let mut p = f.parent_id;
        let mut seen = HashSet::new();
        while let Some(id) = p {
            if !seen.insert(id) {
                break;
            }
            d += 1;
            p = by_id.get(&id).and_then(|x| x.parent_id);
        }
        d
    };
    let mut upserts: Vec<(usize, FolderOp)> = Vec::new();
    for f in locals.iter().filter(|f| f.role == FolderRole::Custom) {
        match shadow_of.get(&f.id) {
            None if f.remote_id.is_none() => upserts.push((depth(f), FolderOp::Create(f.id))),
            Some(s) if FolderState::from_value(&s.state) != FolderState::of(f) => upserts.push((depth(f), FolderOp::Update(f.id))),
            _ => {}
        }
    }
    upserts.sort_by_key(|(d, op)| (*d, matches!(op, FolderOp::Update(_))));
    let mut deletes: Vec<FolderOp> = shadows
        .iter()
        .filter(|(id, _)| !by_id.contains_key(id))
        .map(|(id, s)| FolderOp::Delete { local_id: *id, remote: s.remote_id.clone() })
        .collect();
    deletes.sort_by_key(|op| match op {
        FolderOp::Delete { remote, .. } => std::cmp::Reverse(remote.len()),
        _ => std::cmp::Reverse(0),
    });
    upserts.into_iter().map(|(_, op)| op).chain(deletes).collect()
}

/// Server drafts that were sent from here: their local message left Drafts
/// for Outbox / Sent and is no longer a draft. The server draft must be
/// deleted (not moved: the server or the app files the real Sent copy).
pub fn sent_drafts(locals: &HashMap<axentra_core::MessageId, Message>, draft_shadows: &[(i64, Shadow)], sent_like: &HashSet<FolderId>) -> Vec<(i64, Shadow)> {
    draft_shadows
        .iter()
        .filter(|(id, _)| locals.get(id).is_some_and(|m| !m.is_draft && sent_like.contains(&m.folder_id)))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------- other items

/// Generic outcome for events, contacts, tasks and lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemOp {
    None,
    Create,
    Update,
    Delete,
}

/// Compare a local item's fingerprint with its shadow's.
pub fn item_op(local: Option<&Value>, shadow: Option<&Shadow>) -> ItemOp {
    match (local, shadow) {
        (Some(_), None) => ItemOp::Create,
        (None, Some(_)) => ItemOp::Delete,
        (None, None) => ItemOp::None,
        (Some(fp), Some(s)) if *fp != s.state => ItemOp::Update,
        _ => ItemOp::None,
    }
}

/// Writable event fields (my response and deleted occurrences are tracked
/// separately because they map to different server calls).
pub fn event_fingerprint(e: &Event) -> Value {
    json!({
        "title": e.title, "location": e.location, "body": e.body_html,
        "start": e.start.timestamp(), "end": e.end.timestamp(), "all_day": e.all_day,
        "recurrence": e.recurrence, "show_as": e.show_as, "private": e.is_private,
        "reminder": e.reminder_minutes, "attendees": e.attendees.iter().map(|a| (&a.email.address, a.kind)).collect::<Vec<_>>(),
        "categories": e.categories, "online": e.is_online_meeting,
    })
}

/// Server-side extras tracked for an event.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EventExtra {
    pub response: ResponseStatus,
    pub exdates: Vec<NaiveDate>,
    /// Set on exceptions (modified occurrences) of a series.
    pub series_master_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventOp {
    Create,
    Update,
    Respond(ResponseStatus),
    /// Occurrences the user deleted locally from a series.
    CancelOccurrences(Vec<NaiveDate>),
    Delete,
}

pub fn event_ops(local: Option<&Event>, shadow: Option<&Shadow>) -> Vec<EventOp> {
    let fp = local.map(event_fingerprint);
    match item_op(fp.as_ref(), shadow) {
        ItemOp::Create => return vec![EventOp::Create],
        ItemOp::Delete => return vec![EventOp::Delete],
        _ => {}
    }
    let (Some(e), Some(s)) = (local, shadow) else { return Vec::new() };
    let extra: EventExtra = serde_json::from_value(s.extra.clone()).unwrap_or_default();
    let mut ops = Vec::new();
    if fp.as_ref() != Some(&s.state) {
        ops.push(EventOp::Update);
    }
    if e.my_response != extra.response && matches!(e.my_response, ResponseStatus::Accepted | ResponseStatus::Tentative | ResponseStatus::Declined) {
        ops.push(EventOp::Respond(e.my_response));
    }
    let added: Vec<NaiveDate> = e.exdates.iter().filter(|d| !extra.exdates.contains(d)).copied().collect();
    if !added.is_empty() {
        ops.push(EventOp::CancelOccurrences(added));
    }
    ops
}

pub fn contact_fingerprint(c: &Contact) -> Value {
    json!({
        "given": c.given_name, "surname": c.surname, "display": c.display_name, "nick": c.nickname,
        "emails": c.emails.iter().map(|e| &e.value).collect::<Vec<_>>(), "phones": c.phones, "addresses": c.addresses,
        "websites": c.websites, "company": c.company, "title": c.job_title, "dept": c.department,
        "office": c.office_location, "birthday": c.birthday, "notes": c.notes,
    })
}

pub fn task_fingerprint(t: &Task) -> Value {
    json!({
        "title": t.title, "notes": t.notes, "done": t.is_completed, "important": t.is_important, "due": t.due,
        "reminder": t.reminder.map(|r| r.timestamp()), "recurrence": t.recurrence, "categories": t.categories,
        "steps": t.steps.iter().map(|s| (&s.title, s.is_completed)).collect::<Vec<_>>(),
    })
}

pub fn task_list_fingerprint(l: &TaskList) -> Value {
    json!({ "name": l.name })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axentra_core::{Event, Message};

    fn msg() -> Message {
        Message { id: 7, folder_id: 1, is_read: false, flag: FlagStatus::NotFlagged, ..Default::default() }
    }

    #[test]
    fn flag_read_and_move_diffs_become_push_ops() {
        let last = MessageState::of(&msg());
        assert!(message_ops(Some(&msg()), &last).is_empty());

        let mut m = msg();
        m.is_read = true;
        m.flag = FlagStatus::Flagged;
        m.folder_id = 4;
        assert_eq!(
            message_ops(Some(&m), &last),
            vec![MessageOp::Move(4), MessageOp::Read(true), MessageOp::Flag(FlagStatus::Flagged, None)]
        );

        // Completing a flag and changing a due date are flag ops too.
        let mut flagged_last = last.clone();
        flagged_last.flag = FlagStatus::Flagged;
        let mut done = msg();
        done.flag = FlagStatus::Complete;
        assert_eq!(message_ops(Some(&done), &flagged_last), vec![MessageOp::Flag(FlagStatus::Complete, None)]);
        let mut due = msg();
        due.flag = FlagStatus::Flagged;
        due.flag_due = NaiveDate::from_ymd_opt(2026, 10, 30);
        assert_eq!(message_ops(Some(&due), &flagged_last), vec![MessageOp::Flag(FlagStatus::Flagged, due.flag_due)]);

        let mut other = msg();
        other.inference = InferenceClass::Other;
        other.categories = vec!["Red category".into()];
        assert_eq!(
            message_ops(Some(&other), &last),
            vec![MessageOp::Inference(InferenceClass::Other), MessageOp::Categories(vec!["Red category".into()])]
        );
        assert_eq!(message_ops(None, &last), vec![MessageOp::Delete]);
    }

    #[test]
    fn server_changes_merge_without_clobbering_local_edits() {
        let mut last = MessageState::of(&msg());
        let mut local = msg();
        local.flag = FlagStatus::Flagged; // user flagged it, not yet pushed
        let server = MessageState { is_read: true, flag: FlagStatus::NotFlagged, folder_id: 1, ..Default::default() };
        assert!(merge_server_message(&mut local, &mut last, &server));
        assert!(local.is_read, "untouched field takes the server value");
        assert_eq!(local.flag, FlagStatus::Flagged, "local edit survives");
        assert_eq!(last, server);
        assert_eq!(message_ops(Some(&local), &last), vec![MessageOp::Flag(FlagStatus::Flagged, None)]);

        // Server moved it (Graph keeps the id across moves).
        let moved = MessageState { folder_id: 9, ..server.clone() };
        assert!(merge_server_message(&mut local, &mut last, &moved));
        assert_eq!(local.folder_id, 9);
    }

    #[test]
    fn drafts_sent_from_here_are_found() {
        let shadow = |uid: u32| Shadow { remote_id: format!("1:{uid}:Drafts"), container: "Drafts".into(), uid: Some(uid), ..Default::default() };
        let mut locals = HashMap::new();
        locals.insert(1, Message { id: 1, folder_id: 10, is_draft: true, ..Default::default() }); // still a draft
        locals.insert(2, Message { id: 2, folder_id: 11, is_draft: false, ..Default::default() }); // in Outbox
        locals.insert(3, Message { id: 3, folder_id: 12, is_draft: false, ..Default::default() }); // in Sent
        let shadows = vec![(1, shadow(5)), (2, shadow(6)), (3, shadow(7)), (4, shadow(8))];
        let sent_like: HashSet<FolderId> = [11, 12].into_iter().collect();
        let found: Vec<i64> = sent_drafts(&locals, &shadows, &sent_like).into_iter().map(|(id, _)| id).collect();
        assert_eq!(found, vec![2, 3], "deleted drafts (4) are ordinary tombstones");
    }

    #[test]
    fn folder_changes_become_ordered_ops() {
        let folder = |id: FolderId, parent: Option<FolderId>, name: &str, remote: Option<&str>| Folder {
            id,
            parent_id: parent,
            name: name.into(),
            remote_id: remote.map(str::to_string),
            ..Default::default()
        };
        let shadow = |remote: &str, name: &str, parent: Option<FolderId>| Shadow {
            remote_id: remote.into(),
            state: FolderState { name: name.into(), parent }.to_value(),
            ..Default::default()
        };
        let inbox = Folder { id: 1, role: FolderRole::Inbox, name: "Inbox".into(), remote_id: Some("INBOX".into()), ..Default::default() };
        let locals = vec![
            inbox,
            folder(5, Some(4), "Child", None),           // new, under a new parent
            folder(4, None, "Projects", None),           // new
            folder(2, None, "Receipts 2026", Some("Receipts")), // renamed here
            folder(3, Some(1), "Travel", Some("Travel")), // moved under Inbox
            folder(6, None, "Same", Some("Same")),       // unchanged
        ];
        let shadows = vec![
            (2, shadow("Receipts", "Receipts", None)),
            (3, shadow("Travel", "Travel", None)),
            (6, shadow("Same", "Same", None)),
            (7, shadow("Old", "Old", None)),           // deleted here
            (8, shadow("Old/Sub", "Sub", Some(7))),    // deleted here (child)
        ];
        assert_eq!(
            folder_ops(&locals, &shadows),
            vec![
                FolderOp::Create(4),
                FolderOp::Update(2),
                FolderOp::Create(5),
                FolderOp::Update(3),
                FolderOp::Delete { local_id: 8, remote: "Old/Sub".into() },
                FolderOp::Delete { local_id: 7, remote: "Old".into() },
            ]
        );
        // Well-known folders never produce ops; synced folders without a shadow
        // (from before folder tracking) wait for the next pull.
        assert!(folder_ops(&[folder(9, None, "Legacy", Some("Legacy"))], &[]).is_empty());
    }

    #[test]
    fn event_ops_cover_create_update_respond_and_occurrences() {
        let e = Event { id: 1, title: "Standup".into(), ..Default::default() };
        assert_eq!(event_ops(Some(&e), None), vec![EventOp::Create]);
        let shadow = Shadow { remote_id: "AAMk".into(), state: event_fingerprint(&e), ..Default::default() };
        assert!(event_ops(Some(&e), Some(&shadow)).is_empty());
        let mut changed = e.clone();
        changed.title = "Daily standup".into();
        changed.my_response = ResponseStatus::Accepted;
        changed.exdates = vec![NaiveDate::from_ymd_opt(2026, 10, 21).unwrap()];
        assert_eq!(
            event_ops(Some(&changed), Some(&shadow)),
            vec![
                EventOp::Update,
                EventOp::Respond(ResponseStatus::Accepted),
                EventOp::CancelOccurrences(vec![NaiveDate::from_ymd_opt(2026, 10, 21).unwrap()])
            ]
        );
        assert_eq!(event_ops(None, Some(&shadow)), vec![EventOp::Delete]);
        assert_eq!(item_op(None, None), ItemOp::None);
    }
}
