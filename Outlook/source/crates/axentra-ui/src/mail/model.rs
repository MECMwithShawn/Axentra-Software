//! The message list as data: rows (messages or conversations), group
//! headers and the expanded conversation, computed from the store whenever
//! mail, the folder, the view or the search changes.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use axentra_core::*;
use chrono::{DateTime, Local, TimeZone, Utc};
use dioxus::prelude::ReadableExt;

use super::ctx::{ListView, MailCtx, Open, PersonView, RowKey, SearchScope, SortField};
use crate::state::{AppState, Module, store};

/// Search results are capped like Outlook's (first 1,000).
const SEARCH_LIMIT: u32 = 1000;

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: RowKey,
    /// Newest message of the row.
    pub latest: Message,
    /// Messages the row acts on (a conversation: its messages in this folder).
    pub ids: Vec<MessageId>,
    /// Messages in the whole conversation (badge when > 1).
    pub count: i64,
    pub unread: usize,
    /// Line 1: sender(s), or recipients in Sent Items and Drafts.
    pub people: String,
    pub flag: FlagStatus,
    pub pinned: bool,
    pub has_attachments: bool,
    pub importance: Importance,
    pub mentions_me: bool,
    pub meeting: Option<MeetingMessageKind>,
    pub categories: Vec<String>,
    pub draft: bool,
    /// Folder name shown on search results.
    pub folder_tag: Option<String>,
    /// Attachment preview chips (Roomy).
    pub attachments: Vec<Attachment>,
    /// The user's last response: `Some(false)` replied, `Some(true)` forwarded.
    pub responded: Option<bool>,
    pub open: Open,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChildRow {
    pub key: RowKey,
    pub msg: Message,
    /// Folder name when the message lives in another folder.
    pub folder_tag: Option<String>,
    pub conversation: String,
    pub last: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Header { label: String, count: usize, collapsed: bool },
    Row(Rc<Row>),
    Child(Rc<ChildRow>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListModel {
    pub entries: Vec<Entry>,
    /// Selectable rows in display order.
    pub keys: Vec<RowKey>,
    /// Position of each key in `keys`.
    pub positions: HashMap<RowKey, usize>,
    pub ids: HashMap<RowKey, Vec<MessageId>>,
    pub opens: HashMap<RowKey, Open>,
    /// Group header labels in order ("Jump to").
    pub groups: Vec<String>,
    /// Top-level rows, including those in collapsed groups.
    pub total: usize,
    pub searching: bool,
    /// Words to highlight in subjects and previews while searching.
    pub highlight: Vec<String>,
    pub folder: Option<Folder>,
    /// A favourite person's mail is shown instead of a folder.
    pub person: Option<PersonView>,
    pub conversations: bool,
    /// Category name to preset colour (for the pills).
    pub category_colors: HashMap<String, String>,
    /// Unread mail reached Other since the user last looked at it (shown
    /// as "New messages in Other" above the Focused list).
    pub other_has_new: bool,
    pub error: Option<String>,
}

impl ListModel {
    pub fn index_of(&self, key: &RowKey) -> Option<usize> {
        self.positions.get(key).copied()
    }

    fn push_key(&mut self, key: RowKey) {
        self.positions.insert(key.clone(), self.keys.len());
        self.keys.push(key);
    }

    /// Key of the filter / sort settings for what the list shows: the
    /// folder, or a favourite person (negative contact id).
    pub fn view_key(&self) -> Option<FolderId> {
        match &self.person {
            Some(p) if !self.searching => Some(-p.contact),
            _ => self.folder.as_ref().map(|f| f.id),
        }
    }

    /// A model with just these selectable rows (tests and helpers).
    pub fn from_keys(keys: Vec<RowKey>) -> Self {
        let mut m = ListModel::default();
        for k in keys {
            m.push_key(k);
        }
        m
    }

    /// Messages for a set of rows (deduplicated, in row order).
    pub fn ids_for(&self, keys: &[RowKey]) -> Vec<MessageId> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for k in keys {
            let ids = match self.ids.get(k) {
                Some(ids) => ids.clone(),
                None => match k {
                    RowKey::Msg(id) => vec![*id],
                    RowKey::Conv(_) => Vec::new(),
                },
            };
            for id in ids {
                if seen.insert(id) {
                    out.push(id);
                }
            }
        }
        out
    }

    pub fn row(&self, key: &RowKey) -> Option<&Rc<Row>> {
        self.entries.iter().find_map(|e| match e {
            Entry::Row(r) if &r.key == key => Some(r),
            _ => None,
        })
    }

    /// The message a row stands for (newest of a conversation).
    pub fn message(&self, key: &RowKey) -> Option<Message> {
        self.entries.iter().find_map(|e| match e {
            Entry::Row(r) if &r.key == key => Some(r.latest.clone()),
            Entry::Child(c) if &c.key == key => Some(c.msg.clone()),
            _ => None,
        })
    }
}

/// Size buckets used when sorting by size (classic Outlook ranges).
pub fn size_bucket(bytes: i64) -> &'static str {
    const KB: i64 = 1024;
    const MB: i64 = 1024 * 1024;
    match bytes {
        b if b > 5 * MB => "Enormous (> 5 MB)",
        b if b > MB => "Huge (1 - 5 MB)",
        b if b > 500 * KB => "Very large (500 KB - 1 MB)",
        b if b > 100 * KB => "Large (100 - 500 KB)",
        b if b > 25 * KB => "Medium (25 - 100 KB)",
        b if b > 10 * KB => "Small (10 - 25 KB)",
        _ => "Tiny (< 10 KB)",
    }
}

/// Words and phrases of a search to highlight: free text plus `subject:`
/// and `body:` values, lower-cased (docs/spec/mail.md §7.5).
pub fn highlight_terms(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (key, term) in parse_search(query) {
        let free = key.is_none() && !matches!(term.as_str(), "AND" | "OR" | "NOT");
        if free || matches!(key.as_deref(), Some("subject" | "body" | "content")) {
            let t = term.trim_end_matches('*').trim().to_lowercase();
            if !t.is_empty() && !out.contains(&t) {
                out.push(t);
            }
        }
    }
    // Longest first so a phrase wins over a word inside it.
    out.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
    out
}

/// Split `text` into (segment, highlighted) runs. Terms match at word starts
/// and as prefixes, case-insensitively, like Outlook search.
pub fn split_highlight(text: &str, terms: &[String]) -> Vec<(String, bool)> {
    if terms.is_empty() || text.is_empty() {
        return vec![(text.to_string(), false)];
    }
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let terms: Vec<Vec<char>> = terms.iter().map(|t| t.chars().collect()).collect();
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut push = |s: String, hl: bool| match out.last_mut() {
        Some((prev, h)) if *h == hl => prev.push_str(&s),
        _ => out.push((s, hl)),
    };
    let mut i = 0;
    while i < chars.len() {
        let word_start = i == 0 || !lower[i - 1].is_alphanumeric();
        let hit = if word_start { terms.iter().find(|t| !t.is_empty() && lower[i..].starts_with(t)).map(|t| t.len()) } else { None };
        match hit {
            Some(n) => {
                push(chars[i..i + n].iter().collect(), true);
                i += n;
            }
            None => {
                push(chars[i].to_string(), false);
                i += 1;
            }
        }
    }
    out
}

/// Subject without RE: / FW: prefixes.
pub fn bare_subject(subject: &str) -> &str {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        match ["re:", "fw:", "fwd:"].iter().find(|p| lower.starts_with(*p)) {
            Some(p) => s = s[p.len()..].trim_start(),
            None => return s,
        }
    }
}

fn first_name(a: &EmailAddress) -> String {
    a.display().split_whitespace().next().unwrap_or_default().to_string()
}

fn names(list: &[EmailAddress]) -> String {
    list.iter().map(|a| a.display().to_string()).collect::<Vec<_>>().join("; ")
}

/// Line 1 of a row: recipients in Drafts / Sent Items, otherwise the
/// sender, or the participants' first names for a conversation.
pub fn people_line(latest: &Message, conv: &[Message], role: Option<FolderRole>) -> String {
    if latest.is_draft || role == Some(FolderRole::Drafts) {
        return if latest.to.is_empty() { "(No recipients)".into() } else { names(&latest.to) };
    }
    if role == Some(FolderRole::Sent) {
        return format!("To: {}", names(&latest.to));
    }
    let mut senders: Vec<&EmailAddress> = Vec::new();
    let mut by_newest: Vec<&Message> = conv.iter().collect();
    by_newest.sort_by_key(|m| std::cmp::Reverse(m.received_at));
    for m in by_newest {
        if !m.is_draft && !senders.iter().any(|s| s.address.eq_ignore_ascii_case(&m.from.address)) {
            senders.push(&m.from);
        }
    }
    match senders.as_slice() {
        [] => latest.from.display().to_string(),
        [one] => one.display().to_string(),
        many => many.iter().map(|a| first_name(a)).collect::<Vec<_>>().join("; "),
    }
}

/// Date header for the Date sort; "This Month" keeps the captured casing
/// (docs/spec/mail.md §3.4), future items count as Today.
pub fn date_label(t: DateTime<Utc>, now: DateTime<Local>, week_start: chrono::Weekday) -> &'static str {
    match crate::fmt::date_group(t, now, week_start) {
        "Upcoming" => "Today",
        "This month" => "This Month",
        other => other,
    }
}

/// Group header for a row. Pinned rows get their own group only in a
/// folder sorted newest first (not in search results or a person's mail).
fn group_label(row: &Row, view: &ListView, pinned_group: bool, now: DateTime<Local>, week_start: chrono::Weekday, size: i64) -> String {
    match view.sort {
        SortField::Date => {
            if pinned_group && row.pinned && view.descending {
                "Pinned".into()
            } else {
                date_label(row.latest.received_at, now, week_start).into()
            }
        }
        SortField::From => row.latest.from.display().to_string(),
        SortField::Subject => bare_subject(&row.latest.subject)
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().collect::<String>())
            .unwrap_or_else(|| "(No subject)".into()),
        SortField::Category => row.categories.first().cloned().unwrap_or_else(|| "No category".into()),
        SortField::Size => size_bucket(size).into(),
        SortField::Importance => match row.importance {
            Importance::High => "High",
            Importance::Normal => "Normal",
            Importance::Low => "Low",
        }
        .into(),
        SortField::Flag => match row.flag {
            FlagStatus::Flagged => "Flagged",
            FlagStatus::Complete => "Completed",
            FlagStatus::NotFlagged => "Not flagged",
        }
        .into(),
    }
}

/// Approximate message size in bytes (attachments plus text), for the
/// Size sort.
fn approx_size(m: &Message, attachments: &HashMap<MessageId, Vec<Attachment>>) -> i64 {
    let att: i64 = attachments.get(&m.id).map(|v| v.iter().map(|a| a.size).sum()).unwrap_or(0);
    att + 2048 + m.preview.len() as i64 * 4
}

fn sort_rank(row: &Row, field: SortField) -> (i64, String) {
    match field {
        SortField::From => (0, row.latest.from.display().to_lowercase()),
        SortField::Subject => (0, bare_subject(&row.latest.subject).to_lowercase()),
        SortField::Category => match row.categories.first() {
            Some(c) => (0, c.to_lowercase()),
            None => (1, String::new()),
        },
        SortField::Importance => (
            match row.importance {
                Importance::High => 0,
                Importance::Normal => 1,
                Importance::Low => 2,
            },
            String::new(),
        ),
        SortField::Flag => (
            match row.flag {
                FlagStatus::Flagged => 0,
                FlagStatus::Complete => 1,
                FlagStatus::NotFlagged => 2,
            },
            String::new(),
        ),
        SortField::Date | SortField::Size => (0, String::new()),
    }
}

/// Inputs read from signals, so `compute` itself is plain code.
pub struct Inputs {
    pub folder_id: Option<FolderId>,
    pub tab: InferenceClass,
    pub view: ListView,
    pub search: Option<String>,
    pub scope: SearchScope,
    pub conversation_view: bool,
    pub focused_inbox: bool,
    pub attachment_chips: bool,
    pub expanded: Option<String>,
    pub collapsed: HashSet<String>,
    pub now: DateTime<Local>,
    pub week_start: chrono::Weekday,
    /// When the user last looked at Other (`None`: not this session).
    pub other_seen: Option<DateTime<Utc>>,
    /// Show a favourite person's mail (Favorites in the folder pane).
    pub person: Option<PersonView>,
}

/// Read every input (subscribing the memo to it) and build the model.
pub fn build(app: AppState, ctx: MailCtx) -> ListModel {
    let _ = app.mail_rev();
    let settings = app.settings.read().clone();
    let search = app.search.read().clone();
    let searching = search.active && (app.module)() == Module::Mail && !search.query.trim().is_empty();
    let folder_id = (ctx.folder)();
    let person = (ctx.person)();
    let view_key = person.as_ref().filter(|_| !searching).map(|p| -p.contact).or(folder_id);
    let today = (ctx.today)();
    let now = if today == Local::now().date_naive() {
        Local::now()
    } else {
        Local.from_local_datetime(&today.and_hms_opt(12, 0, 0).unwrap_or_default()).single().unwrap_or_else(Local::now)
    };
    let inputs = Inputs {
        folder_id,
        tab: (ctx.tab)(),
        view: ctx.view(view_key),
        search: searching.then(|| search.query.trim().to_string()),
        scope: (ctx.search_scope)(),
        conversation_view: settings.conversation_view,
        focused_inbox: settings.focused_inbox,
        attachment_chips: settings.show_attachment_previews && settings.density == Density::Roomy,
        expanded: (ctx.expanded)(),
        collapsed: ctx.collapsed_groups.read().clone(),
        now,
        week_start: settings.week_start,
        other_seen: (ctx.other_seen)(),
        person,
    };
    match compute(&inputs) {
        Ok(m) => m,
        Err(e) => ListModel {
            searching,
            error: Some(e.to_string()),
            folder: folder_id.and_then(|f| store().folder(f).ok().flatten()),
            ..Default::default()
        },
    }
}

pub fn compute(inp: &Inputs) -> Result<ListModel> {
    let s = store();
    let folders = s.folders(None)?;
    let by_id: HashMap<FolderId, &Folder> = folders.iter().map(|f| (f.id, f)).collect();
    let searching = inp.search.is_some();
    // A favourite person's mail (from every folder but Deleted Items and
    // Junk Email) replaces the folder, like a search with its own title.
    let person = inp.person.clone().filter(|_| !searching);
    let across = searching || person.is_some();
    let folder = inp.folder_id.filter(|_| person.is_none()).and_then(|id| by_id.get(&id).map(|f| (*f).clone()));
    let role = folder.as_ref().map(|f| f.role);
    let view = inp.view;

    let store_sort = match (view.sort, view.descending) {
        (SortField::Date, true) => MessageSort::DateNewest,
        (SortField::Date, false) => MessageSort::DateOldest,
        (SortField::Size, _) => MessageSort::DateNewest,
        _ => MessageSort::DateOldest,
    };
    let mut query = MessageQuery { filter: view.filter, sort: store_sort, ..Default::default() };
    if let Some(q) = &inp.search {
        query.search = Some(q.clone());
        query.limit = Some(SEARCH_LIMIT);
        if inp.scope == SearchScope::CurrentFolder {
            query.folder_id = inp.folder_id;
        }
    } else if let Some(p) = &person {
        query.search = Some(format!("from:\"{}\"", p.address.replace('"', "")));
        query.limit = Some(SEARCH_LIMIT);
    } else {
        let Some(f) = &folder else { return Ok(ListModel { folder, ..Default::default() }) };
        query.folder_id = Some(f.id);
        if inp.focused_inbox && f.role == FolderRole::Inbox {
            query.inference = Some(inp.tab);
        }
    }
    // Conversations only group the date-sorted folder views (as in Outlook).
    let conversations = inp.conversation_view && !across && view.sort == SortField::Date && role != Some(FolderRole::Drafts);
    let mut groups: Vec<Vec<Message>> = if conversations {
        s.conversation_groups(&query)?
    } else {
        s.messages(&query)?.into_iter().map(|m| vec![m]).collect()
    };
    if person.is_some() {
        groups.retain(|g| !matches!(by_id.get(&g[0].folder_id).map(|f| f.role), Some(FolderRole::Deleted | FolderRole::Junk)));
    }

    // Conversation sizes across folders, and the full conversations that
    // span folders (for participants), each in one query.
    let conv_ids: Vec<String> = if conversations { groups.iter().map(|g| g[0].conversation_id.clone()).collect() } else { Vec::new() };
    let sizes = s.conversation_sizes(&conv_ids)?;
    let spanning: Vec<String> = groups
        .iter()
        .filter(|g| sizes.get(&g[0].conversation_id).is_some_and(|n| *n > g.len() as i64))
        .map(|g| g[0].conversation_id.clone())
        .collect();
    let full_convs = s.conversations(&spanning)?;
    let need_attachments = inp.attachment_chips || view.sort == SortField::Size;
    let att_ids: Vec<MessageId> = if need_attachments {
        groups.iter().flat_map(|g| g.iter().filter(|m| m.has_attachments).map(|m| m.id)).collect()
    } else {
        Vec::new()
    };
    let attachments = s.attachments_for(&att_ids)?;
    // Replied / forwarded icons (not for the user's own Sent Items and Drafts).
    let responses = if matches!(role, Some(FolderRole::Sent | FolderRole::Drafts)) && !across {
        HashMap::new()
    } else {
        let all_ids: Vec<MessageId> = groups.iter().flatten().map(|m| m.id).collect();
        s.last_responses(&all_ids)?
    };

    let mut rows: Vec<Row> = Vec::with_capacity(groups.len());
    for g in groups {
        let latest = g.iter().max_by_key(|m| (m.received_at, m.id)).cloned().unwrap_or_default();
        let conv = latest.conversation_id.clone();
        let count = if conversations { sizes.get(&conv).copied().unwrap_or(g.len() as i64).max(g.len() as i64) } else { 1 };
        let full = full_convs.get(&conv).filter(|_| conversations);
        let people = people_line(&latest, full.map(|v| v.as_slice()).unwrap_or(&g), if across { None } else { role });
        let responded = g.iter().filter_map(|m| responses.get(&m.id)).max_by_key(|r| r.at).map(|r| r.forwarded);
        let key = if conversations { RowKey::Conv(conv.clone()) } else { RowKey::Msg(latest.id) };
        let flag = if g.iter().any(|m| m.flag == FlagStatus::Flagged) {
            FlagStatus::Flagged
        } else if g.iter().any(|m| m.flag == FlagStatus::Complete) {
            FlagStatus::Complete
        } else {
            FlagStatus::NotFlagged
        };
        let mut categories: Vec<String> = Vec::new();
        for m in &g {
            for c in &m.categories {
                if !categories.contains(c) {
                    categories.push(c.clone());
                }
            }
        }
        let chips: Vec<Attachment> = if inp.attachment_chips {
            g.iter()
                .filter_map(|m| attachments.get(&m.id))
                .flatten()
                .filter(|a| !a.is_inline)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let open = Open {
            message: latest.id,
            conversation: (inp.conversation_view && !latest.is_draft).then(|| conv.clone()),
            mark: g.iter().map(|m| m.id).collect(),
        };
        rows.push(Row {
            key,
            ids: g.iter().map(|m| m.id).collect(),
            count,
            unread: g.iter().filter(|m| !m.is_read).count(),
            people,
            flag,
            pinned: g.iter().any(|m| m.is_pinned),
            has_attachments: g.iter().any(|m| m.has_attachments),
            importance: if g.iter().any(|m| m.importance == Importance::High) {
                Importance::High
            } else if g.iter().all(|m| m.importance == Importance::Low) {
                Importance::Low
            } else {
                Importance::Normal
            },
            mentions_me: g.iter().any(|m| m.mentions_me),
            meeting: g.iter().find_map(|m| m.meeting_kind),
            categories,
            draft: latest.is_draft,
            folder_tag: across.then(|| by_id.get(&latest.folder_id).map(|f| f.name.clone())).flatten(),
            attachments: chips,
            responded,
            open,
            latest,
        });
    }

    // Client-side ordering for the non-date fields (stable, newest first within a value).
    let sizes_by_row: HashMap<MessageId, i64> = rows.iter().map(|r| (r.latest.id, approx_size(&r.latest, &attachments))).collect();
    match view.sort {
        // Results across folders are in date order; pins only lead in folders.
        SortField::Date if across && view.descending => rows.sort_by_key(|r| std::cmp::Reverse((r.latest.received_at, r.latest.id))),
        SortField::Date => {}
        SortField::Size => {
            rows.sort_by(|a, b| {
                let (sa, sb) = (sizes_by_row[&a.latest.id], sizes_by_row[&b.latest.id]);
                let ord = if view.descending { sb.cmp(&sa) } else { sa.cmp(&sb) };
                ord.then(b.latest.received_at.cmp(&a.latest.received_at))
            });
        }
        field => {
            rows.sort_by(|a, b| {
                let (ra, rb) = (sort_rank(a, field), sort_rank(b, field));
                let natural = ra.cmp(&rb);
                let ord = if view.descending != field.default_descending() { natural.reverse() } else { natural };
                ord.then(b.latest.received_at.cmp(&a.latest.received_at))
            });
        }
    }

    let other_has_new = !searching && inp.focused_inbox && role == Some(FolderRole::Inbox) && inp.tab == InferenceClass::Focused && {
        let unread_other = MessageQuery {
            folder_id: inp.folder_id,
            inference: Some(InferenceClass::Other),
            filter: MessageFilter::Unread,
            limit: Some(50),
            ..Default::default()
        };
        s.messages(&unread_other)?.iter().map(|m| m.received_at).max().is_some_and(|t| inp.other_seen.is_none_or(|seen| t > seen))
    };
    let mut model = ListModel {
        searching,
        highlight: inp.search.as_deref().map(highlight_terms).unwrap_or_default(),
        folder: folder.clone(),
        person,
        conversations,
        total: rows.len(),
        category_colors: s.categories()?.into_iter().map(|c| (c.name, c.color)).collect(),
        other_has_new,
        ..Default::default()
    };
    let mut current: Option<String> = None;
    let mut header_at = 0usize;
    for row in rows {
        let label = group_label(&row, &view, !across, inp.now, inp.week_start, sizes_by_row[&row.latest.id]);
        if current.as_deref() != Some(label.as_str()) {
            header_at = model.entries.len();
            model.entries.push(Entry::Header { label: label.clone(), count: 0, collapsed: inp.collapsed.contains(&label) });
            model.groups.push(label.clone());
            current = Some(label.clone());
        }
        if let Some(Entry::Header { count, .. }) = model.entries.get_mut(header_at) {
            *count += 1;
        }
        let collapsed = inp.collapsed.contains(&label);
        model.ids.insert(row.key.clone(), row.ids.clone());
        model.opens.insert(row.key.clone(), row.open.clone());
        if collapsed {
            continue;
        }
        model.push_key(row.key.clone());
        let expanded = conversations && row.count > 1 && inp.expanded.as_deref() == Some(row.latest.conversation_id.as_str());
        let conv = row.latest.conversation_id.clone();
        model.entries.push(Entry::Row(Rc::new(row)));
        if expanded {
            let hidden_roles: &[FolderRole] = match role {
                Some(FolderRole::Deleted) => &[FolderRole::Junk],
                Some(FolderRole::Junk) => &[FolderRole::Deleted],
                _ => &[FolderRole::Deleted, FolderRole::Junk],
            };
            let mut children: Vec<Message> = s
                .conversation(&conv)?
                .into_iter()
                .filter(|m| by_id.get(&m.folder_id).is_some_and(|f| !hidden_roles.contains(&f.role)))
                .collect();
            children.sort_by_key(|m| std::cmp::Reverse(m.received_at));
            let n = children.len();
            for (i, m) in children.into_iter().enumerate() {
                let key = RowKey::Msg(m.id);
                let folder_tag = (Some(m.folder_id) != inp.folder_id).then(|| by_id.get(&m.folder_id).map(|f| f.name.clone())).flatten();
                model.ids.insert(key.clone(), vec![m.id]);
                model.opens.insert(key.clone(), Open { message: m.id, conversation: Some(conv.clone()), mark: vec![m.id] });
                model.push_key(key.clone());
                model.entries.push(Entry::Child(Rc::new(ChildRow { key, msg: m, folder_tag, conversation: conv.clone(), last: i + 1 == n })));
            }
        }
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(from: (&str, &str), days: i64) -> Message {
        Message {
            from: EmailAddress::new(from.0, from.1),
            to: vec![EmailAddress::new("Alex Morgan", "alex@x.example"), EmailAddress::new("", "ops@x.example")],
            received_at: Utc::now() - chrono::Duration::days(days),
            ..Default::default()
        }
    }

    #[test]
    fn people_line_by_folder_and_conversation() {
        let a = m(("Priya Raman", "priya@x.example"), 2);
        let b = m(("Marcus Lee", "marcus@x.example"), 1);
        assert_eq!(people_line(&b, &[a.clone(), b.clone()], Some(FolderRole::Inbox)), "Marcus; Priya");
        assert_eq!(people_line(&a, std::slice::from_ref(&a), Some(FolderRole::Inbox)), "Priya Raman");
        assert_eq!(people_line(&a, std::slice::from_ref(&a), Some(FolderRole::Sent)), "To: Alex Morgan; ops@x.example");
        let mut d = a.clone();
        d.is_draft = true;
        d.to.clear();
        assert_eq!(people_line(&d, std::slice::from_ref(&d), Some(FolderRole::Drafts)), "(No recipients)");
    }

    #[test]
    fn buckets_and_subjects() {
        assert_eq!(size_bucket(4_000), "Tiny (< 10 KB)");
        assert_eq!(size_bucket(300 * 1024), "Large (100 - 500 KB)");
        assert_eq!(size_bucket(18_000_000), "Enormous (> 5 MB)");
        assert_eq!(bare_subject("RE: FW: Re: Budget"), "Budget");
        assert_eq!(bare_subject("Recap"), "Recap");
    }

    #[test]
    fn date_labels_follow_the_spec_casing() {
        let now = Local.with_ymd_and_hms(2026, 10, 22, 9, 0, 0).unwrap();
        let t = |m: u32, d: u32| Local.with_ymd_and_hms(2026, m, d, 8, 0, 0).unwrap().with_timezone(&Utc);
        assert_eq!(date_label(t(10, 23), now, chrono::Weekday::Sun), "Today");
        assert_eq!(date_label(t(10, 2), now, chrono::Weekday::Sun), "This Month");
        assert_eq!(date_label(t(10, 21), now, chrono::Weekday::Sun), "Yesterday");
    }

    #[test]
    fn model_lookups() {
        let mut model = ListModel::from_keys(vec![RowKey::Conv("c".into()), RowKey::Msg(2)]);
        model.ids.insert(RowKey::Conv("c".into()), vec![1, 2]);
        model.ids.insert(RowKey::Msg(2), vec![2]);
        assert_eq!(model.ids_for(&[RowKey::Conv("c".into()), RowKey::Msg(2), RowKey::Msg(7)]), vec![1, 2, 7]);
        assert_eq!(model.index_of(&RowKey::Msg(2)), Some(1));
        assert_eq!(model.index_of(&RowKey::Msg(7)), None);
    }

    #[test]
    fn search_terms_highlight_word_prefixes() {
        let terms = highlight_terms(r#"budget from:wei subject:"q3 plan" NOT ray*"#);
        assert_eq!(terms, vec!["q3 plan", "budget", "ray"]);
        let runs = split_highlight("Ray's Q3 plan: budgets for disarray", &terms);
        assert_eq!(
            runs,
            vec![
                ("Ray".to_string(), true),
                ("'s ".to_string(), false),
                ("Q3 plan".to_string(), true),
                (": ".to_string(), false),
                ("budget".to_string(), true),
                ("s for disarray".to_string(), false),
            ]
        );
        assert_eq!(split_highlight("nothing here", &[]), vec![("nothing here".to_string(), false)]);
    }
}
