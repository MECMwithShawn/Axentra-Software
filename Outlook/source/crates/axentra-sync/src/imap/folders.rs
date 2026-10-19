//! Folder discovery (`LIST`) and role mapping (§5.4): SPECIAL-USE
//! attributes first, then well-known names; one folder per role.

use std::collections::{HashMap, HashSet};

use async_imap::imap_proto::{MailboxDatum, NameAttribute, Response};
use axentra_core::{AccountId, Folder, FolderId, FolderRole, Store};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use super::{Imap, Transport, imap_err, quote, raw_command, raw_command_with};
use crate::diff::{FolderOp, FolderState, KIND_FOLDER, KIND_MESSAGE, Shadow, folder_ops};
use crate::error::{SyncError, SyncResult};

/// Gmail-style virtual folders that are not roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    /// `\All` (Gmail "All Mail"): shown as a normal folder, never Archive.
    All,
    /// `\Flagged` (Gmail "Starred"): hidden.
    Flagged,
    /// `\Important`: hidden.
    Important,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteFolder {
    /// Name exactly as the server sent it (modified UTF-7 if non-ASCII).
    pub name: String,
    pub delimiter: Option<String>,
    /// Decoded last path segment.
    pub display: String,
    /// Raw name of the parent mailbox, if any.
    pub parent: Option<String>,
    /// Lower-cased attributes, e.g. `\sent`, `\noselect`.
    pub attributes: Vec<String>,
    pub role: Option<FolderRole>,
    pub special: Option<Special>,
    pub selectable: bool,
}

/// Per-folder sync cursor (store `sync_folders`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderCursor {
    pub uidvalidity: Option<u32>,
    pub uidnext: Option<u32>,
    pub highest_modseq: Option<u64>,
    /// Lowest UID inside the sync window.
    pub synced_min_uid: Option<u32>,
    pub delimiter: Option<String>,
    /// Graph delta link for this folder.
    pub delta_link: Option<String>,
}

/// Decode an RFC 3501 modified UTF-7 mailbox name.
pub fn decode_mutf7(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let Some(dash) = after.find('-') else {
            out.push_str(&rest[amp..]);
            return out;
        };
        let chunk = &after[..dash];
        if chunk.is_empty() {
            out.push('&');
        } else {
            let b64 = chunk.replace(',', "/");
            match base64::engine::general_purpose::STANDARD_NO_PAD.decode(b64.as_bytes()) {
                Ok(bytes) if bytes.len() % 2 == 0 => {
                    let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    out.push_str(&String::from_utf16_lossy(&units));
                }
                _ => {
                    out.push('&');
                    out.push_str(chunk);
                    out.push('-');
                }
            }
        }
        rest = &after[dash + 1..];
    }
    out.push_str(rest);
    out
}

/// Encode a name as modified UTF-7 (for CREATE).
pub fn encode_mutf7(s: &str) -> String {
    let mut out = String::new();
    let mut pending: Vec<u16> = Vec::new();
    let flush = |pending: &mut Vec<u16>, out: &mut String| {
        if pending.is_empty() {
            return;
        }
        let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
        out.push('&');
        out.push_str(&base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes).replace('/', ","));
        out.push('-');
        pending.clear();
    };
    for c in s.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut pending, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

fn attr_name(a: &NameAttribute<'_>) -> String {
    match a {
        NameAttribute::NoInferiors => "\\noinferiors".into(),
        NameAttribute::NoSelect => "\\noselect".into(),
        NameAttribute::Marked => "\\marked".into(),
        NameAttribute::Unmarked => "\\unmarked".into(),
        NameAttribute::All => "\\all".into(),
        NameAttribute::Archive => "\\archive".into(),
        NameAttribute::Drafts => "\\drafts".into(),
        NameAttribute::Flagged => "\\flagged".into(),
        NameAttribute::Junk => "\\junk".into(),
        NameAttribute::Sent => "\\sent".into(),
        NameAttribute::Trash => "\\trash".into(),
        NameAttribute::Extension(e) => e.to_ascii_lowercase(),
        #[allow(unreachable_patterns)]
        _ => String::new(),
    }
}

/// Build a remote folder from a LIST row.
pub fn remote_folder(name: &str, delimiter: Option<&str>, attributes: Vec<String>) -> RemoteFolder {
    let segments: Vec<&str> = match delimiter {
        Some(d) if !d.is_empty() => name.split(d).collect(),
        _ => vec![name],
    };
    let parent = (segments.len() > 1).then(|| segments[..segments.len() - 1].join(delimiter.unwrap_or_default()));
    let display = decode_mutf7(segments.last().copied().unwrap_or(name));
    let has = |a: &str| attributes.iter().any(|x| x == a);
    let selectable = !has("\\noselect") && !has("\\nonexistent");
    let special = if has("\\all") {
        Some(Special::All)
    } else if has("\\flagged") {
        Some(Special::Flagged)
    } else if has("\\important") {
        Some(Special::Important)
    } else {
        None
    };
    RemoteFolder { name: name.to_string(), delimiter: delimiter.map(str::to_string), display, parent, attributes, role: None, special, selectable }
}

fn attribute_role(attrs: &[String]) -> Option<FolderRole> {
    attrs.iter().find_map(|a| match a.as_str() {
        "\\drafts" => Some(FolderRole::Drafts),
        "\\sent" => Some(FolderRole::Sent),
        "\\trash" => Some(FolderRole::Deleted),
        "\\junk" => Some(FolderRole::Junk),
        "\\archive" => Some(FolderRole::Archive),
        _ => None,
    })
}

fn name_role(display: &str) -> Option<FolderRole> {
    let n = display.trim().to_lowercase();
    match n.as_str() {
        "drafts" | "draft" => Some(FolderRole::Drafts),
        "sent items" | "sent" | "sent mail" | "sent messages" => Some(FolderRole::Sent),
        "deleted items" | "trash" | "bin" | "deleted messages" => Some(FolderRole::Deleted),
        "junk email" | "junk" | "spam" | "bulk mail" | "junk e-mail" => Some(FolderRole::Junk),
        "archive" | "archives" => Some(FolderRole::Archive),
        _ => None,
    }
}

/// Give each role to exactly one folder: INBOX, then SPECIAL-USE
/// attributes, then name fallbacks (shallowest folder first).
pub fn assign_roles(folders: &mut [RemoteFolder]) {
    let mut taken: HashSet<FolderRole> = HashSet::new();
    for f in folders.iter_mut() {
        if f.name.eq_ignore_ascii_case("INBOX") && taken.insert(FolderRole::Inbox) {
            f.role = Some(FolderRole::Inbox);
        }
    }
    for f in folders.iter_mut() {
        if f.role.is_none()
            && f.selectable
            && let Some(r) = attribute_role(&f.attributes)
            && taken.insert(r)
        {
            f.role = Some(r);
        }
    }
    let mut order: Vec<usize> = (0..folders.len()).collect();
    order.sort_by_key(|&i| (folders[i].name.matches(folders[i].delimiter.as_deref().unwrap_or("\u{0}")).count(), i));
    for i in order {
        let f = &mut folders[i];
        if f.role.is_none()
            && f.selectable
            && f.special.is_none()
            && let Some(r) = name_role(&f.display)
            && taken.insert(r)
        {
            f.role = Some(r);
        }
    }
}

/// `LIST "" "*"` (with `RETURN (SPECIAL-USE)` when LIST-EXTENDED is there).
pub async fn list<T: Transport>(imap: &mut Imap<T>) -> SyncResult<Vec<RemoteFolder>> {
    let command = if imap.has("LIST-EXTENDED") && imap.has("SPECIAL-USE") {
        "LIST \"\" \"*\" RETURN (SPECIAL-USE)"
    } else {
        "LIST \"\" \"*\""
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    raw_command_with(&mut imap.session, command, |r| {
        if let Response::MailboxData(MailboxDatum::List { name_attributes, delimiter, name }) = r
            && seen.insert(name.to_string())
        {
            out.push(remote_folder(name, delimiter.as_deref(), name_attributes.iter().map(attr_name).collect()));
        }
    })
    .await
    .map_err(|e| match e {
        crate::error::SyncError::Protocol(m) => crate::error::SyncError::Protocol(m),
        other => other,
    })?;
    super::drain_unsolicited(&mut imap.session);
    assign_roles(&mut out);
    Ok(out)
}

/// Create a mailbox for a missing role (`CREATE "Archive" (USE (\Archive))`).
pub async fn create_role_folder<T: Transport>(imap: &mut Imap<T>, role: FolderRole) -> SyncResult<String> {
    let name = role.default_name().to_string();
    let attr = match role {
        FolderRole::Drafts => "\\Drafts",
        FolderRole::Sent => "\\Sent",
        FolderRole::Deleted => "\\Trash",
        FolderRole::Junk => "\\Junk",
        _ => "\\Archive",
    };
    let encoded = encode_mutf7(&name);
    if imap.has("CREATE-SPECIAL-USE") {
        super::raw_command(&mut imap.session, &format!("CREATE {} (USE ({attr}))", super::quote(&encoded))).await?;
    } else {
        imap.session.create(&encoded).await.map_err(|e| imap_err(e, "server"))?;
    }
    Ok(encoded)
}

/// A server folder and the local folder it syncs into.
#[derive(Clone, Debug)]
pub struct MappedFolder {
    pub folder_id: FolderId,
    pub remote: RemoteFolder,
}

/// Mirror the server's folder list into the store: roles attach to the
/// account's well-known folders, other folders are created / renamed /
/// re-parented, and custom folders that vanished from the server are removed.
///
/// Custom folders keep a [`FolderState`] shadow (`sync_items` kind
/// `folder`): a name or parent the user changed here since the last sync
/// is kept (it is pushed by [`push_changes`] / Graph's `push_folders`), and
/// a folder created here is adopted when the server already has one with
/// the same name in the same place.
pub fn apply_to_store(store: &Store, account_id: AccountId, remote: &[RemoteFolder]) -> SyncResult<Vec<MappedFolder>> {
    let mut local = store.folders(Some(account_id))?;
    let mut by_remote: HashMap<String, FolderId> = local.iter().filter_map(|f| f.remote_id.clone().map(|r| (r, f.id))).collect();
    let shadows: HashMap<FolderId, Shadow> = store.sync_list::<Shadow>(account_id, KIND_FOLDER, None)?.into_iter().collect();
    let mut mapped = Vec::new();
    let mut keep: HashSet<FolderId> = HashSet::new();
    let base_order = local.iter().map(|f| f.sort_order).max().unwrap_or(0).max(100);

    // Gmail's "[Gmail]" container and other non-selectable nodes are not shown;
    // their children hang from the closest shown ancestor.
    let shown = |f: &RemoteFolder| f.selectable && !matches!(f.special, Some(Special::Flagged | Special::Important));
    let remote_by_name: HashMap<&str, &RemoteFolder> = remote.iter().map(|r| (r.name.as_str(), r)).collect();

    // Parents before children.
    let depth = |r: &RemoteFolder| {
        let mut d = 0;
        let mut p = r.parent.as_deref();
        while let Some(name) = p {
            d += 1;
            p = remote_by_name.get(name).and_then(|x| x.parent.as_deref());
            if d > 64 {
                break;
            }
        }
        d
    };
    let mut ordered: Vec<&RemoteFolder> = remote.iter().collect();
    ordered.sort_by_key(|r| (depth(r), r.display.to_lowercase()));

    for (i, r) in ordered.into_iter().enumerate() {
        if !shown(r) {
            continue;
        }
        let mut parent_id = None;
        let mut p = r.parent.clone();
        while let Some(pn) = p {
            if let Some(id) = by_remote.get(&pn) {
                parent_id = Some(*id);
                break;
            }
            p = remote_by_name.get(pn.as_str()).and_then(|x| x.parent.clone());
        }
        let existing = match r.role {
            Some(role) => local.iter().find(|f| f.role == role).map(|f| f.id),
            None => by_remote.get(&r.name).copied().or_else(|| {
                // A folder created here that the server already has (or that
                // a previous push created before we could record it).
                local
                    .iter()
                    .find(|f| {
                        f.role == FolderRole::Custom
                            && f.remote_id.is_none()
                            && f.parent_id == parent_id
                            && f.name.eq_ignore_ascii_case(&r.display)
                            && !shadows.contains_key(&f.id)
                    })
                    .map(|f| f.id)
            }),
        };
        let id = match existing.and_then(|id| local.iter_mut().find(|f| f.id == id)) {
            Some(f) => {
                let mut changed = false;
                if f.remote_id.as_deref() != Some(r.name.as_str()) {
                    f.remote_id = Some(r.name.clone());
                    changed = true;
                }
                if r.role.is_none() {
                    // Fields the user changed here since the last sync win.
                    let last = shadows.get(&f.id).map(|s| FolderState::from_value(&s.state));
                    if f.name != r.display && last.as_ref().is_none_or(|l| l.name == f.name) {
                        f.name = r.display.clone();
                        changed = true;
                    }
                    if f.parent_id != parent_id && last.as_ref().is_none_or(|l| l.parent == f.parent_id) {
                        f.parent_id = parent_id;
                        changed = true;
                    }
                }
                if changed {
                    store.save_synced_folder(f)?;
                }
                f.id
            }
            None => {
                // A folder that used to hold this role, now mapped elsewhere, keeps its id.
                let mut f = Folder {
                    account_id,
                    parent_id: if r.role.is_some() { None } else { parent_id },
                    name: if let Some(role) = r.role { role.default_name().into() } else { r.display.clone() },
                    role: r.role.unwrap_or(FolderRole::Custom),
                    remote_id: Some(r.name.clone()),
                    sort_order: base_order + 1 + i as i64,
                    is_expanded: true,
                    ..Default::default()
                };
                let id = store.save_synced_folder(&mut f)?;
                local.push(f);
                id
            }
        };
        by_remote.insert(r.name.clone(), id);
        keep.insert(id);
        if r.role.is_none() {
            let state = FolderState { name: r.display.clone(), parent: parent_id };
            let shadow = Shadow { remote_id: r.name.clone(), state: state.to_value(), ..Default::default() };
            store.sync_put(account_id, KIND_FOLDER, id, &r.name, "", &shadow)?;
        }
        let cursor: FolderCursor = store.sync_folder_get(id)?.unwrap_or_default();
        let cursor = FolderCursor { delimiter: r.delimiter.clone(), ..cursor };
        store.sync_folder_put(id, account_id, &r.name, &cursor)?;
        mapped.push(MappedFolder { folder_id: id, remote: r.clone() });
    }
    // Server-side deletions (and well-known folders the server doesn't have).
    for f in &local {
        if f.remote_id.is_some() && !keep.contains(&f.id) {
            store.remove_synced_folder(f.id)?;
            store.sync_remove(account_id, KIND_FOLDER, f.id)?;
        }
    }
    // Tombstones of folders that are gone on both sides.
    for (id, sh) in &shadows {
        if !keep.contains(id) && !local.iter().any(|f| f.id == *id) && !remote_by_name.contains_key(sh.remote_id.as_str()) {
            store.sync_remove(account_id, KIND_FOLDER, *id)?;
        }
    }
    Ok(mapped)
}

// ---------------------------------------------------------------- pushing folder changes

/// Whether folders were created, renamed, moved or deleted here since the
/// last sync of this account.
pub fn changes_pending(store: &Store, account_id: AccountId) -> SyncResult<bool> {
    let locals = store.folders(Some(account_id))?;
    let shadows = store.sync_list::<Shadow>(account_id, KIND_FOLDER, None)?;
    Ok(!folder_ops(&locals, &shadows).is_empty())
}

/// The personal namespace prefix ("INBOX." on Courier-style servers) when
/// every other mailbox lives under INBOX.
fn personal_prefix(remote: &[RemoteFolder], delimiter: &str) -> String {
    let others: Vec<&RemoteFolder> = remote.iter().filter(|r| !r.name.eq_ignore_ascii_case("INBOX")).collect();
    let prefix = format!("INBOX{delimiter}");
    if !others.is_empty() && others.iter().all(|r| r.name.to_ascii_uppercase().starts_with(&prefix.to_ascii_uppercase())) {
        prefix
    } else {
        String::new()
    }
}

/// Server name for a folder `name` under `parent` (its server name).
pub fn mailbox_name(parent: Option<&str>, name: &str, delimiter: &str, prefix: &str) -> String {
    let leaf = encode_mutf7(&name.replace(delimiter, " "));
    match parent {
        Some(p) => format!("{p}{delimiter}{leaf}"),
        None => format!("{prefix}{leaf}"),
    }
}

fn already_exists(e: &SyncError) -> bool {
    let t = e.to_string().to_ascii_lowercase();
    t.contains("alreadyexists") || t.contains("already exists") || t.contains("mailbox exists")
}

fn nonexistent(e: &SyncError) -> bool {
    let t = e.to_string().to_ascii_lowercase();
    t.contains("nonexistent") || t.contains("doesn't exist") || t.contains("does not exist") || t.contains("not found") || t.contains("no such")
}

/// Push folder creates, renames, moves and deletes made here (CREATE /
/// RENAME / DELETE). `remote` is the current LIST. Returns how many
/// changes reached the server (LIST again when > 0).
pub async fn push_changes<T: Transport>(imap: &mut Imap<T>, store: &Store, account_id: AccountId, remote: &[RemoteFolder]) -> SyncResult<usize> {
    let locals = store.folders(Some(account_id))?;
    let shadows = store.sync_list::<Shadow>(account_id, KIND_FOLDER, None)?;
    let ops = folder_ops(&locals, &shadows);
    if ops.is_empty() {
        return Ok(0);
    }
    let delimiter = remote.iter().find_map(|r| r.delimiter.clone().filter(|d| !d.is_empty())).unwrap_or_else(|| "/".into());
    let prefix = personal_prefix(remote, &delimiter);
    let mut remote_of: HashMap<FolderId, String> = locals.iter().filter_map(|f| f.remote_id.clone().map(|r| (f.id, r))).collect();
    let by_id: HashMap<FolderId, &Folder> = locals.iter().map(|f| (f.id, f)).collect();
    let mut pushed = 0;
    for op in ops {
        match op {
            FolderOp::Create(id) => {
                let Some(f) = by_id.get(&id) else { continue };
                // The parent must exist on the server first.
                let parent = match f.parent_id {
                    Some(p) => match remote_of.get(&p) {
                        Some(r) => Some(r.clone()),
                        None => continue,
                    },
                    None => None,
                };
                let name = mailbox_name(parent.as_deref(), &f.name, &delimiter, &prefix);
                match raw_command(&mut imap.session, &format!("CREATE {}", quote(&name))).await {
                    Ok(_) => {}
                    Err(e) if already_exists(&e) => {}
                    Err(e) => return Err(e),
                }
                let _ = raw_command(&mut imap.session, &format!("SUBSCRIBE {}", quote(&name))).await;
                let mut folder = (*f).clone();
                folder.remote_id = Some(name.clone());
                store.save_synced_folder(&mut folder)?;
                let shadow = Shadow { remote_id: name.clone(), state: FolderState::of(f).to_value(), ..Default::default() };
                store.sync_put(account_id, KIND_FOLDER, id, &name, "", &shadow)?;
                store.sync_folder_put(id, account_id, &name, &FolderCursor { delimiter: Some(delimiter.clone()), ..Default::default() })?;
                remote_of.insert(id, name);
                pushed += 1;
            }
            FolderOp::Update(id) => {
                // Re-read: renaming an ancestor earlier in this pass moved it.
                let (Some(f), Some(sh)) = (by_id.get(&id), store.sync_get::<Shadow>(account_id, KIND_FOLDER, id)?) else { continue };
                let parent = match f.parent_id {
                    Some(p) => match remote_of.get(&p) {
                        Some(r) => Some(r.clone()),
                        None => continue,
                    },
                    None => None,
                };
                let old = sh.remote_id.clone();
                let new = mailbox_name(parent.as_deref(), &f.name, &delimiter, &prefix);
                if new != old {
                    match raw_command(&mut imap.session, &format!("RENAME {} {}", quote(&old), quote(&new))).await {
                        Ok(_) => {}
                        Err(e) if nonexistent(&e) => {
                            // Gone on the server: forget it; the next pull recreates nothing.
                            store.sync_remove(account_id, KIND_FOLDER, id)?;
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                    let _ = raw_command(&mut imap.session, &format!("SUBSCRIBE {}", quote(&new))).await;
                    rename_subtree(store, account_id, &locals, &old, &new, &delimiter)?;
                    let child_prefix = format!("{old}{delimiter}");
                    for r in remote_of.values_mut() {
                        if *r == old {
                            *r = new.clone();
                        } else if let Some(rest) = r.strip_prefix(&child_prefix) {
                            *r = format!("{new}{delimiter}{rest}");
                        }
                    }
                }
                let shadow = Shadow { remote_id: new.clone(), state: FolderState::of(f).to_value(), ..Default::default() };
                store.sync_put(account_id, KIND_FOLDER, id, &new, "", &shadow)?;
                pushed += 1;
            }
            FolderOp::Delete { local_id, remote } => {
                match raw_command(&mut imap.session, &format!("DELETE {}", quote(&remote))).await {
                    Ok(_) => {}
                    Err(e) if nonexistent(&e) => {}
                    Err(e) => return Err(e),
                }
                store.sync_remove(account_id, KIND_FOLDER, local_id)?;
                store.sync_remove_container(account_id, KIND_MESSAGE, &remote)?;
                pushed += 1;
            }
        }
    }
    super::drain_unsolicited(&mut imap.session);
    Ok(pushed)
}

/// After `RENAME old new`: point the folder and its subfolders (and their
/// cursors and message shadows) at the new mailbox names.
fn rename_subtree(store: &Store, account_id: AccountId, locals: &[Folder], old: &str, new: &str, delimiter: &str) -> SyncResult<()> {
    let child_prefix = format!("{old}{delimiter}");
    for f in locals {
        let Some(r) = f.remote_id.as_deref() else { continue };
        let renamed = if r == old {
            new.to_string()
        } else if let Some(rest) = r.strip_prefix(&child_prefix) {
            format!("{new}{delimiter}{rest}")
        } else {
            continue;
        };
        let mut folder = f.clone();
        folder.remote_id = Some(renamed.clone());
        store.save_synced_folder(&mut folder)?;
        let cursor: FolderCursor = store.sync_folder_get(f.id)?.unwrap_or_default();
        store.sync_folder_put(f.id, account_id, &renamed, &cursor)?;
        if let Some(mut sh) = store.sync_get::<Shadow>(account_id, KIND_FOLDER, f.id)? {
            sh.remote_id = renamed.clone();
            store.sync_put(account_id, KIND_FOLDER, f.id, &renamed, "", &sh)?;
        }
        store.sync_rename_container(account_id, KIND_MESSAGE, r, &renamed)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rf(name: &str, attrs: &[&str]) -> RemoteFolder {
        remote_folder(name, Some("/"), attrs.iter().map(|a| a.to_lowercase()).collect())
    }

    #[test]
    fn modified_utf7_round_trip() {
        assert_eq!(decode_mutf7("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode_mutf7("&ZeVnLIqe-"), "日本語");
        assert_eq!(decode_mutf7("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(decode_mutf7("&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-"), "Отправленные");
        for s in ["Entwürfe", "日本語", "Tom & Jerry", "Отправленные", "plain"] {
            assert_eq!(decode_mutf7(&encode_mutf7(s)), s);
        }
        assert_eq!(encode_mutf7("Entwürfe"), "Entw&APw-rfe");
    }

    #[test]
    fn special_use_wins_over_names_and_one_folder_per_role() {
        let mut folders = vec![
            rf("INBOX", &["\\HasNoChildren"]),
            rf("[Gmail]", &["\\Noselect", "\\HasChildren"]),
            rf("[Gmail]/All Mail", &["\\All"]),
            rf("[Gmail]/Sent Mail", &["\\Sent"]),
            rf("[Gmail]/Spam", &["\\Junk"]),
            rf("[Gmail]/Starred", &["\\Flagged"]),
            rf("[Gmail]/Bin", &["\\Trash"]),
            rf("[Gmail]/Drafts", &["\\Drafts"]),
            rf("Sent", &[]),
            rf("Projects/Archive", &[]),
            rf("Archive", &[]),
        ];
        assign_roles(&mut folders);
        let role = |n: &str| folders.iter().find(|f| f.name == n).unwrap().role;
        assert_eq!(role("INBOX"), Some(FolderRole::Inbox));
        assert_eq!(role("[Gmail]/Sent Mail"), Some(FolderRole::Sent));
        assert_eq!(role("Sent"), None, "attribute match takes the role first");
        assert_eq!(role("[Gmail]/Spam"), Some(FolderRole::Junk));
        assert_eq!(role("[Gmail]/Bin"), Some(FolderRole::Deleted));
        assert_eq!(role("[Gmail]/All Mail"), None, "\\All is not Archive");
        assert_eq!(role("Archive"), Some(FolderRole::Archive), "shallowest name match");
        assert_eq!(role("Projects/Archive"), None);
        let all = folders.iter().find(|f| f.name == "[Gmail]/All Mail").unwrap();
        assert_eq!((all.special, all.display.as_str(), all.parent.as_deref()), (Some(Special::All), "All Mail", Some("[Gmail]")));
        assert!(!folders.iter().find(|f| f.name == "[Gmail]").unwrap().selectable);
    }

    #[test]
    fn name_fallbacks_without_special_use() {
        let mut folders = vec![
            remote_folder("INBOX", Some("."), vec![]),
            remote_folder("INBOX.Sent Items", Some("."), vec![]),
            remote_folder("INBOX.Deleted Messages", Some("."), vec![]),
            remote_folder("INBOX.Junk E-mail", Some("."), vec![]),
            remote_folder("INBOX.Drafts", Some("."), vec![]),
            remote_folder("INBOX.Entw&APw-rfe", Some("."), vec![]),
        ];
        assign_roles(&mut folders);
        let roles: Vec<_> = folders.iter().map(|f| f.role).collect();
        assert_eq!(
            roles,
            vec![
                Some(FolderRole::Inbox),
                Some(FolderRole::Sent),
                Some(FolderRole::Deleted),
                Some(FolderRole::Junk),
                Some(FolderRole::Drafts),
                None
            ]
        );
        assert_eq!(folders[5].display, "Entwürfe");
    }

    #[test]
    fn mirror_into_store() {
        let store = Store::open_in_memory().unwrap();
        let mut acct = axentra_core::Account { kind: axentra_core::AccountKind::Imap, email: "a@x".into(), ..Default::default() };
        let id = store.add_account(&mut acct).unwrap();
        let mut remote = vec![
            rf("INBOX", &[]),
            rf("[Gmail]", &["\\Noselect"]),
            rf("[Gmail]/Sent Mail", &["\\Sent"]),
            rf("[Gmail]/All Mail", &["\\All"]),
            rf("[Gmail]/Starred", &["\\Flagged"]),
            rf("Work", &[]),
            rf("Work/Clients", &[]),
        ];
        assign_roles(&mut remote);
        let mapped = apply_to_store(&store, id, &remote).unwrap();
        assert_eq!(mapped.len(), 5, "container and Starred are not shown");
        let folders = store.folders(Some(id)).unwrap();
        let sent = folders.iter().find(|f| f.role == FolderRole::Sent).unwrap();
        assert_eq!((sent.name.as_str(), sent.remote_id.as_deref()), ("Sent Items", Some("[Gmail]/Sent Mail")));
        let work = folders.iter().find(|f| f.name == "Work").unwrap();
        let clients = folders.iter().find(|f| f.name == "Clients").unwrap();
        assert_eq!(clients.parent_id, Some(work.id));
        let all = folders.iter().find(|f| f.name == "All Mail").unwrap();
        assert_eq!(all.parent_id, None);

        // "Work/Clients" deleted on the server, "Work" renamed.
        let mut remote2 = vec![rf("INBOX", &[]), rf("[Gmail]/Sent Mail", &["\\Sent"]), rf("Jobs", &[])];
        assign_roles(&mut remote2);
        apply_to_store(&store, id, &remote2).unwrap();
        let folders = store.folders(Some(id)).unwrap();
        assert!(folders.iter().any(|f| f.name == "Jobs"));
        assert!(!folders.iter().any(|f| f.name == "Clients" || f.name == "Work" || f.name == "All Mail"));
        assert!(folders.iter().any(|f| f.role == FolderRole::Archive), "local well-known folders stay");
    }

    #[test]
    fn local_folder_edits_survive_the_pull_and_new_folders_are_adopted() {
        let store = Store::open_in_memory().unwrap();
        let mut acct = axentra_core::Account { kind: axentra_core::AccountKind::Imap, email: "a@x".into(), ..Default::default() };
        let id = store.add_account(&mut acct).unwrap();
        let mut remote = vec![rf("INBOX", &[]), rf("Receipts", &[]), rf("Travel", &[])];
        assign_roles(&mut remote);
        apply_to_store(&store, id, &remote).unwrap();
        let find = |name: &str| store.folders(Some(id)).unwrap().into_iter().find(|f| f.name == name);
        let receipts = find("Receipts").unwrap();
        let travel = find("Travel").unwrap();
        assert!(!changes_pending(&store, id).unwrap(), "fresh pull: nothing to push");

        // Here: rename Receipts, create Projects. Server: rename Travel.
        store.rename_folder(receipts.id, "Receipts 2026").unwrap();
        let projects = store.create_folder(id, None, "Projects").unwrap();
        assert!(changes_pending(&store, id).unwrap());
        let mut remote = vec![rf("INBOX", &[]), rf("Receipts", &[]), rf("Trips", &[])];
        assign_roles(&mut remote);
        apply_to_store(&store, id, &remote).unwrap();
        assert_eq!(store.folder(receipts.id).unwrap().unwrap().name, "Receipts 2026", "local rename kept until pushed");
        // IMAP has no stable mailbox ids: a rename elsewhere is a new folder.
        assert!(store.folder(travel.id).unwrap().is_none());
        let travel = find("Trips").unwrap();
        assert_eq!(store.folder(projects).unwrap().unwrap().remote_id, None);

        // The server gets a "projects" folder (e.g. created by another client):
        // the local one is adopted instead of duplicated.
        let mut remote = vec![rf("INBOX", &[]), rf("Receipts", &[]), rf("Trips", &[]), rf("projects", &[])];
        assign_roles(&mut remote);
        apply_to_store(&store, id, &remote).unwrap();
        assert_eq!(store.folder(projects).unwrap().unwrap().remote_id.as_deref(), Some("projects"));
        assert_eq!(store.folders(Some(id)).unwrap().iter().filter(|f| f.name.eq_ignore_ascii_case("projects")).count(), 1);

        // A folder deleted here leaves a tombstone until the server agrees.
        store.delete_folder(travel.id).unwrap(); // moves it under Deleted Items
        let deleted = store.folder_by_role(id, FolderRole::Deleted).unwrap().unwrap();
        assert_eq!(store.folder(travel.id).unwrap().unwrap().parent_id, Some(deleted.id));
        let shadows = store.sync_list::<Shadow>(id, KIND_FOLDER, None).unwrap();
        let ops = folder_ops(&store.folders(Some(id)).unwrap(), &shadows);
        assert!(ops.contains(&FolderOp::Update(travel.id)) && ops.contains(&FolderOp::Update(receipts.id)), "{ops:?}");
    }

    #[test]
    fn mailbox_names_for_new_folders() {
        assert_eq!(mailbox_name(None, "Projects", "/", ""), "Projects");
        assert_eq!(mailbox_name(Some("INBOX"), "Clients", ".", ""), "INBOX.Clients");
        assert_eq!(mailbox_name(None, "a/b", "/", ""), "a b", "the delimiter can't be part of a name");
        assert_eq!(mailbox_name(None, "Café", "/", "INBOX/"), "INBOX/Caf&AOk-");
        let remote = vec![rf("INBOX", &[]), rf("INBOX/Sent", &[]), rf("INBOX/Trash", &[])];
        assert_eq!(personal_prefix(&remote, "/"), "INBOX/");
        assert_eq!(personal_prefix(&[rf("INBOX", &[]), rf("Sent", &[])], "/"), "");
    }
}
