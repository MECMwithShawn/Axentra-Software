//! Contacts, contact lists and contact folders, plus vCard / CSV import and
//! export (docs/spec/people-todo.md section 2).
//!
//! Contact folders are plain names (kv key `people.folders`); a contact's
//! `folder` is empty for the default "Contacts" folder. Deletes go through
//! a small trash table so a whole batch can be restored by Undo.

mod csv;
mod text_codec;
mod vcard;

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use super::Store;
use crate::model::*;
use crate::{Error, Result};

pub(crate) const SCHEMA_EXTRA: &str = r#"
CREATE TABLE IF NOT EXISTS people_trash (
    id INTEGER PRIMARY KEY, batch TEXT NOT NULL, kind TEXT NOT NULL, json TEXT NOT NULL, deleted_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS people_trash_batch ON people_trash(batch);
"#;

const FOLDERS_KEY: &str = "people.folders";
const DISMISSED_KEY: &str = "people.suggested_dismissed";
/// `remote_id` prefix of the placeholder contact returned by the batch
/// deletes. The shell's Undo re-inserts it with `insert_contact`, which
/// restores the whole batch instead of inserting the placeholder.
const RESTORE_PREFIX: &str = "axentra:people-restore:";
/// Trash rows older than this are pruned (Undo only lives for seconds).
const TRASH_KEEP_HOURS: i64 = 24;
const IMPORT_BATCH: usize = 100;

fn normalise(contact: &mut Contact) {
    contact.given_name = contact.given_name.trim().to_string();
    contact.surname = contact.surname.trim().to_string();
    contact.display_name = contact.display_name.trim().to_string();
    if contact.display_name.is_empty() {
        contact.display_name = [contact.given_name.as_str(), contact.surname.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
    }
    for e in contact.emails.iter_mut() {
        e.value = e.value.trim().to_string();
    }
    contact.emails.retain(|e| !e.value.is_empty());
    contact.phones.retain(|p| !p.value.trim().is_empty());
    contact.websites.retain(|w| !w.trim().is_empty());
    contact.im_addresses.retain(|w| !w.trim().is_empty());
    if contact.display_name.is_empty() {
        contact.display_name = contact
            .primary_email()
            .map(str::to_string)
            .or_else(|| (!contact.company.is_empty()).then(|| contact.company.clone()))
            .or_else(|| contact.phones.first().map(|p| p.value.trim().to_string()))
            .unwrap_or_default();
    }
}

/// Members deduplicated by address (case-insensitive), empty ones dropped.
fn dedupe_members(members: &mut Vec<EmailAddress>) {
    let mut seen = HashSet::new();
    members.retain(|m| {
        let a = m.address.trim().to_lowercase();
        !a.is_empty() && seen.insert(a)
    });
    for m in members.iter_mut() {
        m.address = m.address.trim().to_string();
        m.name = m.name.trim().to_string();
    }
}

/// Key used to recognise duplicates on import: normalised display name and
/// primary email (docs/spec/people-todo.md 2.13).
fn duplicate_key(c: &Contact) -> (String, String) {
    let name = c.display_name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    (name, c.primary_email().unwrap_or_default().to_lowercase())
}

fn restore_token(batch: &str) -> Contact {
    Contact { display_name: "Deleted items".into(), remote_id: Some(format!("{RESTORE_PREFIX}{batch}")), ..Default::default() }
}

fn folders_in(conn: &Connection) -> Result<Vec<String>> {
    let json: Option<String> = conn.query_row("SELECT value FROM kv WHERE key = ?1", [FOLDERS_KEY], |r| r.get(0)).optional()?;
    Ok(json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
}

fn set_folders_in(conn: &Connection, folders: &[String]) -> Result<()> {
    let mut sorted = folders.to_vec();
    sorted.sort_by_key(|f| f.to_lowercase());
    conn.execute(
        "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![FOLDERS_KEY, serde_json::to_string(&sorted)?],
    )?;
    Ok(())
}

fn is_noreply(address: &str) -> bool {
    let local = address.split('@').next().unwrap_or_default().to_lowercase();
    ["noreply", "no-reply", "no_reply", "donotreply", "do-not-reply", "mailer-daemon", "postmaster", "notification", "bounce"]
        .iter()
        .any(|p| local.contains(p))
}

impl Store {
    /// All contacts, sorted by display name (case-insensitive).
    pub fn contacts(&self) -> Result<Vec<Contact>> {
        self.query_docs("ORDER BY display_name COLLATE NOCASE, id", [])
    }

    pub fn contact(&self, id: ContactId) -> Result<Option<Contact>> {
        self.get_doc(id)
    }

    /// Number of contacts plus contact lists (no documents are loaded).
    pub fn people_count(&self) -> Result<usize> {
        let n: i64 = self.conn().query_row("SELECT (SELECT COUNT(*) FROM contacts) + (SELECT COUNT(*) FROM contact_lists)", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// Categories carried by at least one contact, sorted case-insensitively
    /// (the category nodes of the navigation pane).
    pub fn contact_categories_in_use(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT je.value FROM contacts, json_each(json_extract(contacts.json, '$.categories')) je
              WHERE je.type = 'text' AND je.value <> '' ORDER BY je.value COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Number of contacts in a folder ("" = the default "Contacts" folder).
    pub fn contacts_in_folder_count(&self, folder: &str) -> Result<usize> {
        let n: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM contacts WHERE coalesce(json_extract(json, '$.folder'), '') = ?1",
            [folder],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Contacts with these ids, in the given order (missing ids are skipped).
    pub fn contacts_by_ids(&self, ids: &[ContactId]) -> Result<Vec<Contact>> {
        let conn = self.conn();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(c) = Self::get_doc_in::<Contact>(&conn, *id)? {
                out.push(c);
            }
        }
        Ok(out)
    }

    /// Contact owning this email address, if any (case-insensitive).
    pub fn contact_by_email(&self, address: &str) -> Result<Option<Contact>> {
        Ok(self
            .query_docs::<Contact>(
                "WHERE EXISTS (SELECT 1 FROM json_each(json_extract(contacts.json, '$.emails')) je
                  WHERE lower(json_extract(je.value, '$.value')) = lower(?1)) LIMIT 1",
                [address.trim()],
            )?
            .into_iter()
            .next())
    }

    pub fn search_contacts(&self, q: &str) -> Result<Vec<Contact>> {
        let pat = format!("%{}%", q.trim());
        self.query_docs(
            "WHERE display_name LIKE ?1 OR json_extract(json,'$.emails') LIKE ?1 OR json_extract(json,'$.company') LIKE ?1
             OR json_extract(json,'$.phones') LIKE ?1 OR json_extract(json,'$.nickname') LIKE ?1
             OR json_extract(json,'$.notes') LIKE ?1 ORDER BY display_name COLLATE NOCASE",
            [pat],
        )
    }

    /// Insert a contact. Display name is derived from the name fields (or the
    /// email / company / phone) when left blank. `created_at` is kept when
    /// already set (re-created contacts keep their "Recently added" place).
    ///
    /// A placeholder returned by [`Store::delete_people`] restores that whole
    /// batch instead, and the first restored contact's id is returned.
    pub fn insert_contact(&self, contact: &mut Contact) -> Result<ContactId> {
        if let Some(batch) = contact.remote_id.as_deref().and_then(|r| r.strip_prefix(RESTORE_PREFIX)) {
            let batch = batch.to_string();
            return self.restore_batch(&batch);
        }
        normalise(contact);
        if contact.display_name.is_empty() {
            return Err(Error::Invalid("add a name or an email address".into()));
        }
        if contact.account_id == 0 {
            contact.account_id = self.primary_account()?.map(|a| a.id).ok_or_else(|| Error::NotFound("account".into()))?;
        }
        let now = Utc::now();
        if contact.created_at.timestamp() <= 0 {
            contact.created_at = now;
        }
        contact.updated_at = now;
        self.insert_doc(contact)
    }

    pub fn update_contact(&self, contact: &Contact) -> Result<()> {
        let mut c = contact.clone();
        normalise(&mut c);
        if c.display_name.is_empty() {
            return Err(Error::Invalid("add a name or an email address".into()));
        }
        c.updated_at = Utc::now();
        self.update_doc(&c)
    }

    /// Delete one contact permanently (no trash; see [`Store::delete_people`]).
    pub fn delete_contact(&self, id: ContactId) -> Result<()> {
        self.delete_doc::<Contact>(id)
    }

    pub fn set_contact_favorite(&self, id: ContactId, favorite: bool) -> Result<()> {
        self.set_favorites(&[id], &[], favorite)
    }

    /// Mark contacts and contact lists as favourites (or not). Removing the
    /// flag also forgets the item's place in the Favorites order.
    pub fn set_favorites(&self, contacts: &[ContactId], lists: &[ContactListId], favorite: bool) -> Result<()> {
        self.tx(|tx| {
            for id in contacts {
                let mut c: Contact = Self::get_doc_in(tx, *id)?.ok_or_else(|| Error::NotFound(format!("contact {id}")))?;
                c.is_favorite = favorite;
                if !favorite {
                    c.favorite_order = None;
                }
                Self::update_doc_in(tx, &c)?;
            }
            for id in lists {
                let mut l: ContactList = Self::get_doc_in(tx, *id)?.ok_or_else(|| Error::NotFound(format!("contact list {id}")))?;
                l.is_favorite = favorite;
                if !favorite {
                    l.favorite_order = None;
                }
                Self::update_doc_in(tx, &l)?;
            }
            Ok(())
        })
    }

    /// Persist a dragged Favorites order: `order` lists (is_list, id) top to bottom.
    pub fn reorder_favorites(&self, order: &[(bool, i64)]) -> Result<()> {
        self.tx(|tx| {
            for (pos, (is_list, id)) in order.iter().enumerate() {
                if *is_list {
                    if let Some(mut l) = Self::get_doc_in::<ContactList>(tx, *id)? {
                        l.favorite_order = Some(pos as i64);
                        Self::update_doc_in(tx, &l)?;
                    }
                } else if let Some(mut c) = Self::get_doc_in::<Contact>(tx, *id)? {
                    c.favorite_order = Some(pos as i64);
                    Self::update_doc_in(tx, &c)?;
                }
            }
            Ok(())
        })
    }

    /// Outlook's category toggle for a selection: if any contact lacks the
    /// category it is added to all of them, otherwise removed from all.
    pub fn toggle_contact_category(&self, ids: &[ContactId], category: &str) -> Result<()> {
        self.tx(|tx| {
            let mut contacts = Vec::new();
            for id in ids {
                if let Some(c) = Self::get_doc_in::<Contact>(tx, *id)? {
                    contacts.push(c);
                }
            }
            let add = contacts.iter().any(|c| !c.categories.iter().any(|x| x == category));
            for mut c in contacts {
                c.categories.retain(|x| x != category);
                if add {
                    c.categories.push(category.to_string());
                }
                Self::update_doc_in(tx, &c)?;
            }
            Ok(())
        })
    }

    /// Remove a category from every contact; returns how many changed.
    pub fn remove_category_from_contacts(&self, category: &str) -> Result<usize> {
        let ids: Vec<ContactId> =
            self.contacts()?.into_iter().filter(|c| c.categories.iter().any(|x| x == category)).map(|c| c.id).collect();
        self.tx(|tx| {
            for id in &ids {
                if let Some(mut c) = Self::get_doc_in::<Contact>(tx, *id)? {
                    c.categories.retain(|x| x != category);
                    Self::update_doc_in(tx, &c)?;
                }
            }
            Ok(ids.len())
        })
    }

    /// Move contacts to a folder ("" = the default "Contacts" folder).
    pub fn move_contacts(&self, ids: &[ContactId], folder: &str) -> Result<()> {
        self.tx(|tx| {
            if !folder.is_empty() && !folders_in(tx)?.iter().any(|f| f == folder) {
                return Err(Error::NotFound(format!("contact folder {folder}")));
            }
            for id in ids {
                if let Some(mut c) = Self::get_doc_in::<Contact>(tx, *id)? {
                    c.folder = folder.to_string();
                    Self::update_doc_in(tx, &c)?;
                }
            }
            Ok(())
        })
    }

    /// Delete contacts and contact lists (never the members of a list), keeping
    /// a snapshot. Returns a placeholder contact for
    /// `UndoAction::RestoreContact`: inserting it restores the whole batch with
    /// the original ids.
    pub fn delete_people(&self, contacts: &[ContactId], lists: &[ContactListId]) -> Result<Contact> {
        let batch = uuid::Uuid::new_v4().to_string();
        self.tx(|tx| {
            let now = Utc::now().timestamp();
            tx.execute("DELETE FROM people_trash WHERE deleted_at < ?1", [now - TRASH_KEEP_HOURS * 3600])?;
            for id in contacts {
                if let Some(c) = Self::get_doc_in::<Contact>(tx, *id)? {
                    tx.execute(
                        "INSERT INTO people_trash (batch, kind, json, deleted_at) VALUES (?1, 'contact', ?2, ?3)",
                        params![batch, serde_json::to_string(&c)?, now],
                    )?;
                    Self::delete_doc_in::<Contact>(tx, *id)?;
                }
            }
            for id in lists {
                if let Some(l) = Self::get_doc_in::<ContactList>(tx, *id)? {
                    tx.execute(
                        "INSERT INTO people_trash (batch, kind, json, deleted_at) VALUES (?1, 'list', ?2, ?3)",
                        params![batch, serde_json::to_string(&l)?, now],
                    )?;
                    Self::delete_doc_in::<ContactList>(tx, *id)?;
                }
            }
            Ok(())
        })?;
        Ok(restore_token(&batch))
    }

    /// Re-create everything deleted in `batch`. Returns the first contact's id
    /// (0 when the batch held only lists or folders).
    fn restore_batch(&self, batch: &str) -> Result<ContactId> {
        self.tx(|tx| {
            let rows: Vec<(String, String)> = {
                let mut stmt = tx.prepare("SELECT kind, json FROM people_trash WHERE batch = ?1 ORDER BY id")?;
                let mapped = stmt.query_map([batch], |r| Ok((r.get(0)?, r.get(1)?)))?;
                mapped.collect::<rusqlite::Result<_>>()?
            };
            if rows.is_empty() {
                return Err(Error::NotFound("nothing to restore".into()));
            }
            let mut first = 0;
            for (kind, json) in rows {
                match kind.as_str() {
                    "contact" => {
                        let mut c: Contact = serde_json::from_str(&json)?;
                        if Self::get_doc_in::<Contact>(tx, c.id)?.is_some() {
                            c.id = 0;
                        }
                        let id = Self::insert_doc_in(tx, &mut c)?;
                        if first == 0 {
                            first = id;
                        }
                    }
                    "list" => {
                        let mut l: ContactList = serde_json::from_str(&json)?;
                        if Self::get_doc_in::<ContactList>(tx, l.id)?.is_some() {
                            l.id = 0;
                        }
                        Self::insert_doc_in(tx, &mut l)?;
                    }
                    "folder" => {
                        let name: String = serde_json::from_str(&json)?;
                        let mut folders = folders_in(tx)?;
                        if !folders.iter().any(|f| f.eq_ignore_ascii_case(&name)) {
                            folders.push(name);
                            set_folders_in(tx, &folders)?;
                        }
                    }
                    _ => {}
                }
            }
            tx.execute("DELETE FROM people_trash WHERE batch = ?1", [batch])?;
            Ok(first)
        })
    }

    // ------------------------------------------------------------ folders

    /// User contact folders (the default "Contacts" folder is not listed).
    pub fn contact_folders(&self) -> Result<Vec<String>> {
        folders_in(&self.conn())
    }

    /// Create a folder; a taken name gets " (2)", " (3)", ... appended.
    /// Returns the name actually used.
    pub fn create_contact_folder(&self, name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("folder name is empty".into()));
        }
        self.tx(|tx| {
            let mut folders = folders_in(tx)?;
            let taken = |n: &str, folders: &[String]| n.eq_ignore_ascii_case("Contacts") || folders.iter().any(|f| f.eq_ignore_ascii_case(n));
            let mut candidate = name.to_string();
            let mut n = 2;
            while taken(&candidate, &folders) {
                candidate = format!("{name} ({n})");
                n += 1;
            }
            folders.push(candidate.clone());
            set_folders_in(tx, &folders)?;
            Ok(candidate)
        })
    }

    pub fn rename_contact_folder(&self, old: &str, new: &str) -> Result<String> {
        let new = new.trim();
        if new.is_empty() {
            return Err(Error::Invalid("folder name is empty".into()));
        }
        if old == new {
            return Ok(new.to_string());
        }
        let ids: Vec<ContactId> = self.contacts()?.into_iter().filter(|c| c.folder == old).map(|c| c.id).collect();
        self.tx(|tx| {
            let mut folders = folders_in(tx)?;
            let Some(pos) = folders.iter().position(|f| f == old) else {
                return Err(Error::NotFound(format!("contact folder {old}")));
            };
            if new.eq_ignore_ascii_case("Contacts") || folders.iter().any(|f| f != old && f.eq_ignore_ascii_case(new)) {
                return Err(Error::Invalid(format!("a folder named \"{new}\" already exists")));
            }
            folders[pos] = new.to_string();
            set_folders_in(tx, &folders)?;
            for id in &ids {
                if let Some(mut c) = Self::get_doc_in::<Contact>(tx, *id)? {
                    c.folder = new.to_string();
                    Self::update_doc_in(tx, &c)?;
                }
            }
            Ok(new.to_string())
        })
    }

    /// Delete a folder and the contacts in it. Returns an Undo placeholder
    /// (see [`Store::delete_people`]).
    pub fn delete_contact_folder(&self, name: &str) -> Result<Contact> {
        let ids: Vec<ContactId> = self.contacts()?.into_iter().filter(|c| c.folder == name).map(|c| c.id).collect();
        let token = self.delete_people(&ids, &[])?;
        let batch = token.remote_id.as_deref().and_then(|r| r.strip_prefix(RESTORE_PREFIX)).unwrap_or_default().to_string();
        self.tx(|tx| {
            let mut folders = folders_in(tx)?;
            folders.retain(|f| f != name);
            set_folders_in(tx, &folders)?;
            tx.execute(
                "INSERT INTO people_trash (batch, kind, json, deleted_at) VALUES (?1, 'folder', ?2, ?3)",
                params![batch, serde_json::to_string(name)?, Utc::now().timestamp()],
            )?;
            Ok(())
        })?;
        Ok(token)
    }

    // ------------------------------------------------------------ contact lists

    pub fn contact_lists(&self) -> Result<Vec<ContactList>> {
        self.query_docs("ORDER BY json_extract(json,'$.name') COLLATE NOCASE", [])
    }

    pub fn contact_list(&self, id: ContactListId) -> Result<Option<ContactList>> {
        self.get_doc(id)
    }

    pub fn insert_contact_list(&self, list: &mut ContactList) -> Result<ContactListId> {
        list.name = list.name.trim().to_string();
        if list.name.is_empty() {
            return Err(Error::Invalid("contact list name is empty".into()));
        }
        if list.account_id == 0 {
            list.account_id = self.primary_account()?.map(|a| a.id).ok_or_else(|| Error::NotFound("account".into()))?;
        }
        if list.created_at.timestamp() <= 0 {
            list.created_at = Utc::now();
        }
        dedupe_members(&mut list.members);
        self.insert_doc(list)
    }

    pub fn update_contact_list(&self, list: &ContactList) -> Result<()> {
        let mut l = list.clone();
        l.name = l.name.trim().to_string();
        if l.name.is_empty() {
            return Err(Error::Invalid("contact list name is empty".into()));
        }
        dedupe_members(&mut l.members);
        self.update_doc(&l)
    }

    /// Delete a list. Its members are only references, so no contact changes.
    pub fn delete_contact_list(&self, id: ContactListId) -> Result<()> {
        self.delete_doc::<ContactList>(id)
    }

    /// Add members to a list. Returns the addresses that were already in it
    /// (they are ignored).
    pub fn add_to_contact_list(&self, id: ContactListId, members: &[EmailAddress]) -> Result<Vec<String>> {
        self.tx(|tx| {
            let mut list: ContactList = Self::get_doc_in(tx, id)?.ok_or_else(|| Error::NotFound(format!("contact list {id}")))?;
            let mut duplicates = Vec::new();
            for m in members {
                let address = m.address.trim();
                if address.is_empty() {
                    continue;
                }
                if list.members.iter().any(|x| x.address.eq_ignore_ascii_case(address)) {
                    duplicates.push(address.to_string());
                } else {
                    list.members.push(EmailAddress::new(m.name.trim(), address));
                }
            }
            Self::update_doc_in(tx, &list)?;
            Ok(duplicates)
        })
    }

    pub fn remove_from_contact_list(&self, id: ContactListId, address: &str) -> Result<()> {
        self.tx(|tx| {
            let mut list: ContactList = Self::get_doc_in(tx, id)?.ok_or_else(|| Error::NotFound(format!("contact list {id}")))?;
            list.members.retain(|m| !m.address.eq_ignore_ascii_case(address.trim()));
            Self::update_doc_in(tx, &list)
        })
    }

    // ------------------------------------------------------------ mail-derived

    /// The newest messages sent from or to any of `addresses` (Deleted Items,
    /// Junk, Drafts and Outbox excluded), for "Recent emails".
    pub fn recent_messages_with(&self, addresses: &[String], limit: usize) -> Result<Vec<Message>> {
        let addrs: Vec<String> = addresses.iter().map(|a| a.trim().to_lowercase()).filter(|a| !a.is_empty()).collect();
        if addrs.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let marks = (1..=addrs.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(",");
        let tail = format!(
            "WHERE is_draft = 0
               AND folder_id NOT IN (SELECT id FROM folders WHERE role IN ('deleted', 'junk', 'drafts', 'outbox'))
               AND (lower(json_extract(json, '$.from.address')) IN ({marks})
                 OR EXISTS (SELECT 1 FROM json_each(json_extract(messages.json, '$.to')) je
                            WHERE lower(json_extract(je.value, '$.address')) IN ({marks}))
                 OR EXISTS (SELECT 1 FROM json_each(json_extract(messages.json, '$.cc')) je
                            WHERE lower(json_extract(je.value, '$.address')) IN ({marks})))
             ORDER BY received_at DESC, id DESC LIMIT {limit}"
        );
        self.query_docs(&tail, params_from_iter(addrs.iter()))
    }

    /// People the user emailed (or got replies from) since `since` who are
    /// not saved as contacts, most frequent first: the "Suggested" node.
    /// Own addresses, no-reply senders and dismissed people are left out.
    pub fn suggested_people(&self, since: DateTime<Utc>, limit: usize) -> Result<Vec<(EmailAddress, i64)>> {
        let rows: Vec<(Option<String>, Option<String>)> = {
            let conn = self.conn();
            let mut stmt = conn.prepare_cached(
                "SELECT json_extract(je.value, '$.name'), json_extract(je.value, '$.address')
                   FROM messages m JOIN folders f ON f.id = m.folder_id, json_each(json_extract(m.json, '$.to')) je
                  WHERE f.role = 'sent' AND m.received_at >= ?1
                 UNION ALL
                 SELECT json_extract(je.value, '$.name'), json_extract(je.value, '$.address')
                   FROM messages m JOIN folders f ON f.id = m.folder_id, json_each(json_extract(m.json, '$.cc')) je
                  WHERE f.role = 'sent' AND m.received_at >= ?1
                 UNION ALL
                 SELECT json_extract(m.json, '$.from.name'), json_extract(m.json, '$.from.address')
                   FROM messages m JOIN folders f ON f.id = m.folder_id
                  WHERE m.received_at >= ?1 AND json_extract(m.json, '$.in_reply_to') IS NOT NULL
                    AND f.role NOT IN ('sent', 'drafts', 'junk', 'deleted')",
            )?;
            let mapped = stmt.query_map([since.timestamp()], |r| Ok((r.get(0)?, r.get(1)?)))?;
            mapped.collect::<rusqlite::Result<_>>()?
        };
        let mut known: HashSet<String> = self
            .contacts()?
            .iter()
            .flat_map(|c| c.emails.iter().map(|e| e.value.to_lowercase()))
            .collect();
        known.extend(self.accounts()?.into_iter().map(|a| a.email.to_lowercase()));
        known.extend(self.dismissed_suggestions()?);
        let mut counts: HashMap<String, (String, i64)> = HashMap::new();
        for (name, address) in rows {
            let Some(address) = address.map(|a| a.trim().to_string()).filter(|a| a.contains('@')) else { continue };
            let key = address.to_lowercase();
            if known.contains(&key) || is_noreply(&key) {
                continue;
            }
            let entry = counts.entry(key).or_insert_with(|| (String::new(), 0));
            if entry.0.is_empty() {
                entry.0 = name.unwrap_or_default().trim().to_string();
            }
            entry.1 += 1;
        }
        let mut out: Vec<(EmailAddress, i64)> =
            counts.into_iter().map(|(address, (name, n))| (EmailAddress::new(name, address), n)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.display().to_lowercase().cmp(&b.0.display().to_lowercase())));
        out.truncate(limit);
        Ok(out)
    }

    fn dismissed_suggestions(&self) -> Result<Vec<String>> {
        Ok(self.kv_get(DISMISSED_KEY)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
    }

    /// Hide a person from "Suggested" for good.
    pub fn dismiss_suggested_person(&self, address: &str) -> Result<()> {
        let mut list = self.dismissed_suggestions()?;
        let a = address.trim().to_lowercase();
        if !list.contains(&a) {
            list.push(a);
        }
        self.kv_set(DISMISSED_KEY, &serde_json::to_string(&list)?)
    }

    // ------------------------------------------------------------ import / export

    /// Decode a base64-encoded import file to text. The flag is true when the
    /// file was not UTF-8 and was read as Windows-1252.
    pub fn decode_import_file(base64: &str) -> Result<(String, bool)> {
        let bytes = text_codec::base64_decode(base64).ok_or_else(|| Error::Invalid("the file could not be read".into()))?;
        Ok(text_codec::decode_bytes(&bytes))
    }

    /// Parse a `.vcf` document (vCard 2.1, 3.0 or 4.0) into contacts and lists.
    pub fn parse_vcard(text: &str) -> (Vec<Contact>, Vec<ContactList>) {
        vcard::parse(text)
    }

    /// Serialise to vCard 4.0 (`v4`) or 3.0.
    pub fn contacts_to_vcard(contacts: &[Contact], lists: &[ContactList], v4: bool) -> String {
        vcard::serialize(contacts, lists, v4)
    }

    /// Parse an Outlook-style contacts CSV (header row first).
    pub fn parse_contacts_csv(text: &str) -> Vec<Contact> {
        csv::parse(text)
    }

    /// Outlook-style contacts CSV with a UTF-8 BOM.
    pub fn contacts_to_csv(contacts: &[Contact]) -> String {
        csv::serialize(contacts)
    }

    /// Year stored for birthdays entered without a year.
    pub fn birthday_no_year() -> i32 {
        vcard::NO_YEAR
    }

    /// Import parsed contacts and lists into `folder` ("" = Contacts), in
    /// transactions of 100. A contact is skipped when one with the same
    /// normalised display name and primary email exists (or appeared earlier
    /// in the file); a list is skipped when a list with its name exists.
    /// Returns (imported, skipped).
    pub fn import_contacts(&self, contacts: Vec<Contact>, lists: Vec<ContactList>, folder: &str) -> Result<(usize, usize)> {
        let (imported, _, skipped) = self.import_contacts_merging(contacts, lists, folder, false)?;
        Ok((imported, skipped))
    }

    /// [`Store::import_contacts`], optionally merging by email: with
    /// `merge_by_email`, a contact sharing an email address with a saved
    /// contact (or with one earlier in the file) fills in that contact's
    /// empty fields and adds the addresses, numbers, websites and categories
    /// it lacks, instead of being imported again. Returns (imported, merged,
    /// skipped); a merge that adds nothing counts as skipped.
    pub fn import_contacts_merging(
        &self,
        contacts: Vec<Contact>,
        lists: Vec<ContactList>,
        folder: &str,
        merge_by_email: bool,
    ) -> Result<(usize, usize, usize)> {
        let account_id = self.primary_account()?.map(|a| a.id).ok_or_else(|| Error::NotFound("account".into()))?;
        if !folder.is_empty() && !self.contact_folders()?.iter().any(|f| f == folder) {
            return Err(Error::NotFound(format!("contact folder {folder}")));
        }
        let mut saved = self.contacts()?;
        let mut seen: HashSet<(String, String)> = saved.iter().map(duplicate_key).collect();
        let mut list_names: HashSet<String> = self.contact_lists()?.into_iter().map(|l| l.name.to_lowercase()).collect();
        // Lower-case address -> (in `to_insert`?, index), for merging.
        let mut by_email: HashMap<String, (bool, usize)> = HashMap::new();
        if merge_by_email {
            for (i, c) in saved.iter().enumerate() {
                for e in &c.emails {
                    by_email.entry(e.value.to_lowercase()).or_insert((false, i));
                }
            }
        }
        let now = Utc::now();
        let (mut merged, mut skipped) = (0, 0);
        let mut changed: HashSet<usize> = HashSet::new();
        let mut to_insert: Vec<Contact> = Vec::new();
        for mut c in contacts {
            normalise(&mut c);
            if c.display_name.is_empty() {
                skipped += 1;
                continue;
            }
            let target = merge_by_email.then(|| c.emails.iter().find_map(|e| by_email.get(&e.value.to_lowercase()).copied())).flatten();
            if let Some((new, i)) = target {
                let dst = if new { &mut to_insert[i] } else { &mut saved[i] };
                if merge_contact(dst, &c) {
                    merged += 1;
                    if !new {
                        changed.insert(i);
                    }
                    for e in &dst.emails {
                        by_email.entry(e.value.to_lowercase()).or_insert((new, i));
                    }
                } else {
                    skipped += 1;
                }
                continue;
            }
            if !seen.insert(duplicate_key(&c)) {
                skipped += 1;
                continue;
            }
            c.id = 0;
            c.account_id = account_id;
            c.remote_id = None;
            c.folder = folder.to_string();
            c.created_at = now;
            if c.updated_at.timestamp() <= 0 {
                c.updated_at = now;
            }
            if merge_by_email {
                for e in &c.emails {
                    by_email.entry(e.value.to_lowercase()).or_insert((true, to_insert.len()));
                }
            }
            to_insert.push(c);
        }
        let mut imported = to_insert.len();
        for chunk in to_insert.chunks_mut(IMPORT_BATCH) {
            self.tx(|tx| {
                for c in chunk.iter_mut() {
                    Self::insert_doc_in(tx, c)?;
                }
                Ok(())
            })?;
        }
        let mut changed: Vec<usize> = changed.into_iter().collect();
        changed.sort_unstable();
        for chunk in changed.chunks(IMPORT_BATCH) {
            self.tx(|tx| {
                for i in chunk {
                    let c = &mut saved[*i];
                    c.updated_at = now;
                    Self::update_doc_in(tx, c)?;
                }
                Ok(())
            })?;
        }
        for mut l in lists {
            l.name = l.name.trim().to_string();
            if l.name.is_empty() || !list_names.insert(l.name.to_lowercase()) {
                skipped += 1;
                continue;
            }
            l.id = 0;
            l.account_id = account_id;
            l.created_at = now;
            dedupe_members(&mut l.members);
            self.insert_doc(&mut l)?;
            imported += 1;
        }
        Ok((imported, merged, skipped))
    }
}

/// Most email addresses a merge leaves on a contact (Exchange has three
/// slots, docs/spec/people-todo.md 2.8).
const MERGE_MAX_EMAILS: usize = 3;

/// Merge `src` into `dst` without overwriting anything: empty text fields,
/// the birthday and the photo are filled in, and emails, phone numbers (by
/// digits), addresses (one per type), websites, IM addresses and categories
/// that `dst` lacks are added. Returns whether `dst` changed.
fn merge_contact(dst: &mut Contact, src: &Contact) -> bool {
    fn fill(d: &mut String, s: &str) {
        if d.trim().is_empty() && !s.trim().is_empty() {
            *d = s.trim().to_string();
        }
    }
    fn add_missing(d: &mut Vec<String>, s: &[String]) {
        for v in s {
            if !v.trim().is_empty() && !d.iter().any(|x| x.trim().eq_ignore_ascii_case(v.trim())) {
                d.push(v.trim().to_string());
            }
        }
    }
    let before = dst.clone();
    // A display name that only stood in for missing names is recomputed.
    let name_was_fallback = dst.given_name.is_empty() && dst.surname.is_empty();
    fill(&mut dst.given_name, &src.given_name);
    fill(&mut dst.surname, &src.surname);
    fill(&mut dst.middle_name, &src.middle_name);
    if name_was_fallback && (!dst.given_name.is_empty() || !dst.surname.is_empty()) {
        dst.display_name = if src.display_name.trim().is_empty() {
            [dst.given_name.as_str(), dst.surname.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ")
        } else {
            src.display_name.trim().to_string()
        };
    }
    for (d, s) in [
        (&mut dst.nickname, &src.nickname),
        (&mut dst.company, &src.company),
        (&mut dst.job_title, &src.job_title),
        (&mut dst.department, &src.department),
        (&mut dst.office_location, &src.office_location),
        (&mut dst.notes, &src.notes),
    ] {
        fill(d, s);
    }
    let (dm, sm) = (&mut *dst.more, &*src.more);
    for (d, s) in [
        (&mut dm.title, &sm.title),
        (&mut dm.suffix, &sm.suffix),
        (&mut dm.phonetic_given_name, &sm.phonetic_given_name),
        (&mut dm.phonetic_surname, &sm.phonetic_surname),
        (&mut dm.phonetic_company, &sm.phonetic_company),
        (&mut dm.manager, &sm.manager),
        (&mut dm.assistant, &sm.assistant),
        (&mut dm.profession, &sm.profession),
        (&mut dm.spouse, &sm.spouse),
        (&mut dm.children, &sm.children),
        (&mut dm.file_as, &sm.file_as),
    ] {
        fill(d, s);
    }
    if dst.birthday.is_none() {
        dst.birthday = src.birthday;
    }
    if dst.photo.is_none() {
        dst.photo = src.photo.clone();
    }
    for e in &src.emails {
        if dst.emails.len() < MERGE_MAX_EMAILS && !dst.emails.iter().any(|x| x.value.eq_ignore_ascii_case(&e.value)) {
            dst.emails.push(e.clone());
        }
    }
    let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
    for p in &src.phones {
        let d = digits(&p.value);
        if !d.is_empty() && !dst.phones.iter().any(|x| digits(&x.value) == d) {
            dst.phones.push(p.clone());
        }
    }
    for a in &src.addresses {
        if !dst.addresses.iter().any(|x| x.label == a.label) {
            dst.addresses.push(a.clone());
        }
    }
    add_missing(&mut dst.websites, &src.websites);
    add_missing(&mut dst.im_addresses, &src.im_addresses);
    for cat in &src.categories {
        if !dst.categories.contains(cat) {
            dst.categories.push(cat.clone());
        }
    }
    *dst != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_util::store_with_account;
    use chrono::Duration;

    fn email(v: &str) -> Vec<LabeledValue> {
        vec![LabeledValue { label: "Work".into(), value: v.into() }]
    }

    fn add(store: &Store, given: &str, surname: &str, address: &str) -> ContactId {
        let mut c = Contact { given_name: given.into(), surname: surname.into(), emails: email(address), ..Default::default() };
        store.insert_contact(&mut c).unwrap()
    }

    #[test]
    fn display_name_derivation_and_lookup() {
        let (store, _) = store_with_account();
        let mut c = Contact {
            given_name: " Megan ".into(),
            surname: "Bowen".into(),
            emails: vec![LabeledValue { label: "Work".into(), value: "Megan@Contoso.example".into() }, LabeledValue::default()],
            ..Default::default()
        };
        let id = store.insert_contact(&mut c).unwrap();
        let got = store.contact(id).unwrap().unwrap();
        assert_eq!(got.display_name, "Megan Bowen");
        assert_eq!(got.emails.len(), 1);
        assert_eq!(store.contact_by_email("megan@contoso.example").unwrap().unwrap().id, id);
        assert_eq!(store.search_contacts("bow").unwrap().len(), 1);

        let mut only_email = Contact { emails: vec![LabeledValue { label: "".into(), value: "x@y.example".into() }], ..Default::default() };
        store.insert_contact(&mut only_email).unwrap();
        assert_eq!(only_email.display_name, "x@y.example");

        let mut only_phone = Contact { phones: vec![LabeledValue { label: "Mobile".into(), value: " 555 0100 ".into() }], ..Default::default() };
        store.insert_contact(&mut only_phone).unwrap();
        assert_eq!(only_phone.display_name, "555 0100");

        let mut empty = Contact::default();
        assert!(store.insert_contact(&mut empty).is_err());
    }

    #[test]
    fn insert_keeps_existing_created_at() {
        let (store, _) = store_with_account();
        let when = Utc::now() - Duration::days(30);
        let mut c = Contact { display_name: "Old friend".into(), created_at: when, ..Default::default() };
        let id = store.insert_contact(&mut c).unwrap();
        assert_eq!(store.contact(id).unwrap().unwrap().created_at.timestamp(), when.timestamp());
    }

    #[test]
    fn batch_delete_and_restore_keeps_ids() {
        let (store, _) = store_with_account();
        let a = add(&store, "Ada", "Lovelace", "ada@example.com");
        let b = add(&store, "Grace", "Hopper", "grace@example.com");
        let mut list = ContactList { name: "Pioneers".into(), members: vec![EmailAddress::new("Ada", "ada@example.com")], ..Default::default() };
        let l = store.insert_contact_list(&mut list).unwrap();

        let mut token = store.delete_people(&[a, b], &[l]).unwrap();
        assert!(store.contacts().unwrap().is_empty());
        assert!(store.contact_lists().unwrap().is_empty());

        // The shell's Undo re-inserts the placeholder with id 0.
        token.id = 0;
        let first = store.insert_contact(&mut token).unwrap();
        assert_eq!(first, a);
        let ids: Vec<_> = store.contacts().unwrap().iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![a, b]);
        assert_eq!(store.contact_lists().unwrap()[0].id, l);
        // A second undo of the same batch has nothing left to restore.
        assert!(store.insert_contact(&mut token.clone()).is_err());
    }

    #[test]
    fn deleting_a_list_never_deletes_its_members() {
        let (store, _) = store_with_account();
        let a = add(&store, "Ada", "Lovelace", "ada@example.com");
        let mut list = ContactList { name: "Team".into(), members: vec![EmailAddress::new("Ada", "ada@example.com")], ..Default::default() };
        let l = store.insert_contact_list(&mut list).unwrap();
        store.delete_contact_list(l).unwrap();
        assert!(store.contact(a).unwrap().is_some());
        let mut list2 = ContactList { name: "Team 2".into(), members: vec![EmailAddress::new("Ada", "ada@example.com")], ..Default::default() };
        let l2 = store.insert_contact_list(&mut list2).unwrap();
        store.delete_people(&[], &[l2]).unwrap();
        assert!(store.contact(a).unwrap().is_some());
    }

    #[test]
    fn list_membership_dedupes() {
        let (store, _) = store_with_account();
        let mut list = ContactList {
            name: "  Book club ".into(),
            members: vec![EmailAddress::new("Ada", "ada@example.com"), EmailAddress::new("Ada again", "ADA@example.com"), EmailAddress::new("", " ")],
            ..Default::default()
        };
        let id = store.insert_contact_list(&mut list).unwrap();
        let got = store.contact_list(id).unwrap().unwrap();
        assert_eq!(got.name, "Book club");
        assert_eq!(got.members.len(), 1);
        assert!(got.created_at.timestamp() > 0);

        let dups = store
            .add_to_contact_list(id, &[EmailAddress::new("Grace", "grace@example.com"), EmailAddress::new("", "Ada@Example.com")])
            .unwrap();
        assert_eq!(dups, vec!["Ada@Example.com".to_string()]);
        assert_eq!(store.contact_list(id).unwrap().unwrap().members.len(), 2);
        store.remove_from_contact_list(id, "ADA@example.com").unwrap();
        let members = store.contact_list(id).unwrap().unwrap().members;
        assert_eq!(members, vec![EmailAddress::new("Grace", "grace@example.com")]);
        assert!(store.insert_contact_list(&mut ContactList { name: " ".into(), ..Default::default() }).is_err());
    }

    #[test]
    fn favorites_and_categories() {
        let (store, _) = store_with_account();
        let a = add(&store, "Ada", "Lovelace", "ada@example.com");
        let b = add(&store, "Grace", "Hopper", "grace@example.com");
        let mut list = ContactList { name: "L".into(), ..Default::default() };
        let l = store.insert_contact_list(&mut list).unwrap();
        store.set_favorites(&[a, b], &[l], true).unwrap();
        store.reorder_favorites(&[(true, l), (false, b), (false, a)]).unwrap();
        assert_eq!(store.contact(a).unwrap().unwrap().favorite_order, Some(2));
        assert!(store.contact_list(l).unwrap().unwrap().is_favorite);
        store.set_contact_favorite(a, false).unwrap();
        let a_now = store.contact(a).unwrap().unwrap();
        assert!(!a_now.is_favorite);
        assert_eq!(a_now.favorite_order, None);

        store.toggle_contact_category(&[a], "VIP").unwrap();
        // b lacks it, so toggling both adds it to both...
        store.toggle_contact_category(&[a, b], "VIP").unwrap();
        assert!(store.contacts_by_ids(&[a, b]).unwrap().iter().all(|c| c.categories == ["VIP"]));
        // ...and toggling again removes it from both.
        store.toggle_contact_category(&[a, b], "VIP").unwrap();
        assert!(store.contacts_by_ids(&[a, b]).unwrap().iter().all(|c| c.categories.is_empty()));
        store.toggle_contact_category(&[a, b], "Red").unwrap();
        store.toggle_contact_category(&[b], "blue").unwrap();
        assert_eq!(store.contact_categories_in_use().unwrap(), ["blue", "Red"]);
        assert_eq!(store.remove_category_from_contacts("Red").unwrap(), 2);
        assert_eq!(store.contact_categories_in_use().unwrap(), ["blue"]);
    }

    #[test]
    fn counts_without_loading_documents() {
        let (store, _) = store_with_account();
        assert_eq!(store.people_count().unwrap(), 0);
        let a = add(&store, "Ada", "Lovelace", "ada@example.com");
        add(&store, "Grace", "Hopper", "grace@example.com");
        store.insert_contact_list(&mut ContactList { name: "L".into(), ..Default::default() }).unwrap();
        assert_eq!(store.people_count().unwrap(), 3);
        store.create_contact_folder("Family").unwrap();
        store.move_contacts(&[a], "Family").unwrap();
        assert_eq!(store.contacts_in_folder_count("Family").unwrap(), 1);
        assert_eq!(store.contacts_in_folder_count("").unwrap(), 1);
    }

    #[test]
    fn folders_create_rename_move_delete_and_undo() {
        let (store, _) = store_with_account();
        assert_eq!(store.create_contact_folder("Family").unwrap(), "Family");
        assert_eq!(store.create_contact_folder("family").unwrap(), "family (2)");
        assert_eq!(store.create_contact_folder("Contacts").unwrap(), "Contacts (2)");
        assert!(store.create_contact_folder("  ").is_err());
        let a = add(&store, "Ada", "Lovelace", "ada@example.com");
        let b = add(&store, "Grace", "Hopper", "grace@example.com");
        store.move_contacts(&[a, b], "Family").unwrap();
        assert!(store.move_contacts(&[a], "Nope").is_err());

        assert!(store.rename_contact_folder("Family", "family (2)").is_err());
        store.rename_contact_folder("Family", "Relatives").unwrap();
        assert!(store.contact_folders().unwrap().contains(&"Relatives".to_string()));
        assert_eq!(store.contact(a).unwrap().unwrap().folder, "Relatives");

        let mut token = store.delete_contact_folder("Relatives").unwrap();
        assert!(!store.contact_folders().unwrap().contains(&"Relatives".to_string()));
        assert!(store.contacts().unwrap().is_empty());
        token.id = 0;
        store.insert_contact(&mut token).unwrap();
        assert!(store.contact_folders().unwrap().contains(&"Relatives".to_string()));
        assert_eq!(store.contacts().unwrap().len(), 2);
        assert_eq!(store.contact(b).unwrap().unwrap().folder, "Relatives");
    }

    #[test]
    fn import_skips_duplicates_and_lists_by_name() {
        let (store, _) = store_with_account();
        add(&store, "Ada", "Lovelace", "ada@example.com");
        let vcf = "BEGIN:VCARD\nVERSION:3.0\nFN:Ada  Lovelace\nN:Lovelace;Ada;;;\nEMAIL:ADA@example.com\nEND:VCARD\n\
BEGIN:VCARD\nVERSION:3.0\nFN:Ada Lovelace\nEMAIL:ada@other.example\nEND:VCARD\n\
BEGIN:VCARD\nVERSION:3.0\nFN:Grace Hopper\nEMAIL:grace@example.com\nEND:VCARD\n\
BEGIN:VCARD\nVERSION:3.0\nFN:Grace Hopper\nEMAIL:grace@example.com\nEND:VCARD\n\
BEGIN:VCARD\nVERSION:4.0\nKIND:group\nFN:Pioneers\nMEMBER:mailto:grace@example.com\nEND:VCARD\n";
        let (contacts, lists) = Store::parse_vcard(vcf);
        store.create_contact_folder("Imported").unwrap();
        let (imported, skipped) = store.import_contacts(contacts, lists.clone(), "Imported").unwrap();
        assert_eq!((imported, skipped), (3, 2));
        let all = store.contacts().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all.iter().filter(|c| c.folder == "Imported").count(), 2);
        let (again, skipped_again) = store.import_contacts(Vec::new(), lists, "").unwrap();
        assert_eq!((again, skipped_again), (0, 1));
        assert!(store.import_contacts(Vec::new(), Vec::new(), "Missing").is_err());
    }

    #[test]
    fn import_can_merge_by_email() {
        let (store, _) = store_with_account();
        let ada = add(&store, "Ada", "Lovelace", "ada@example.com");
        let mut only_email = Contact { emails: email("grace@example.com"), ..Default::default() };
        let grace = store.insert_contact(&mut only_email).unwrap();
        let vcf = "BEGIN:VCARD\nVERSION:4.0\nFN:Augusta Ada King\nN:King;Augusta;;;\nEMAIL:ADA@example.com\nEMAIL:ada@home.example\n\
TEL;TYPE=cell:+44 20 5550 1815\nORG:Analytical Engines\nNOTE:Imported note\nEND:VCARD\n\
BEGIN:VCARD\nVERSION:4.0\nFN:Grace Hopper\nN:Hopper;Grace;;;\nEMAIL:grace@example.com\nTITLE:Rear Admiral\nEND:VCARD\n\
BEGIN:VCARD\nVERSION:4.0\nFN:Alan Turing\nEMAIL:alan@example.com\nEND:VCARD\n\
BEGIN:VCARD\nVERSION:4.0\nFN:Alan Turing\nEMAIL:alan@example.com\nTEL:+44 161 555 0100\nEND:VCARD\n\
BEGIN:VCARD\nVERSION:4.0\nFN:Ada Lovelace\nEMAIL:ada@example.com\nEND:VCARD\n";
        let (contacts, _) = Store::parse_vcard(vcf);

        // Without merging, only the exact duplicate (Alan's second card) is skipped.
        let (fresh, _) = store_with_account();
        assert_eq!(fresh.import_contacts_merging(contacts.clone(), Vec::new(), "", false).unwrap(), (4, 0, 1));

        let (imported, merged, skipped) = store.import_contacts_merging(contacts, Vec::new(), "", true).unwrap();
        // Alan is new; his second card merges into the first; Ada and Grace merge
        // into the saved contacts; the last card adds nothing to Ada.
        assert_eq!((imported, merged, skipped), (1, 3, 1));
        assert_eq!(store.contacts().unwrap().len(), 3);

        let a = store.contact(ada).unwrap().unwrap();
        assert_eq!((a.given_name.as_str(), a.display_name.as_str()), ("Ada", "Ada Lovelace"), "names are never overwritten");
        assert_eq!(a.emails.iter().map(|e| e.value.as_str()).collect::<Vec<_>>(), ["ada@example.com", "ada@home.example"]);
        assert_eq!((a.company.as_str(), a.notes.as_str()), ("Analytical Engines", "Imported note"));
        assert_eq!(a.phones.len(), 1);

        let g = store.contact(grace).unwrap().unwrap();
        assert_eq!(g.display_name, "Grace Hopper", "a fallback display name gives way to real names");
        assert_eq!(g.job_title, "Rear Admiral");

        let alan = store.contact_by_email("alan@example.com").unwrap().unwrap();
        assert_eq!(alan.phones[0].value, "+44 161 555 0100");
    }

    #[test]
    fn merge_keeps_existing_values_and_limits_emails() {
        let mut dst = Contact {
            given_name: "Ada".into(),
            company: "Old Co".into(),
            emails: vec![
                LabeledValue { label: "Work".into(), value: "a@one.example".into() },
                LabeledValue { label: "Work".into(), value: "a@two.example".into() },
            ],
            phones: vec![LabeledValue { label: "Mobile".into(), value: "+1 (415) 555-0100".into() }],
            addresses: vec![PostalAddress { label: "Home".into(), city: "London".into(), ..Default::default() }],
            categories: vec!["VIP".into()],
            ..Default::default()
        };
        let src = Contact {
            given_name: "Augusta".into(),
            company: "New Co".into(),
            emails: ["a@three.example", "a@four.example"].iter().map(|v| LabeledValue { label: "Work".into(), value: (*v).into() }).collect(),
            phones: vec![LabeledValue { label: "Work".into(), value: "14155550100".into() }],
            addresses: vec![
                PostalAddress { label: "Home".into(), city: "Paris".into(), ..Default::default() },
                PostalAddress { label: "Work".into(), city: "Berlin".into(), ..Default::default() },
            ],
            categories: vec!["VIP".into(), "Research".into()],
            more: Box::new(ContactMore { spouse: "William".into(), ..Default::default() }),
            ..Default::default()
        };
        assert!(merge_contact(&mut dst, &src));
        assert_eq!((dst.given_name.as_str(), dst.company.as_str()), ("Ada", "Old Co"));
        assert_eq!(dst.emails.len(), 3, "three email slots at most");
        assert_eq!(dst.phones.len(), 1, "same digits = same number");
        assert_eq!(dst.addresses.iter().map(|a| a.city.as_str()).collect::<Vec<_>>(), ["London", "Berlin"]);
        assert_eq!(dst.categories, ["VIP", "Research"]);
        assert_eq!(dst.more.spouse, "William");
        let again = dst.clone();
        assert!(!merge_contact(&mut dst, &src), "merging twice changes nothing");
        assert_eq!(dst, again);
    }

    #[test]
    fn export_import_round_trip_through_csv_and_vcard() {
        let (store, _) = store_with_account();
        add(&store, "Ada", "Lovelace", "ada@example.com");
        add(&store, "Grace", "Hopper", "grace@example.com");
        let contacts = store.contacts().unwrap();
        for text in [Store::contacts_to_vcard(&contacts, &[], true), Store::contacts_to_vcard(&contacts, &[], false)] {
            let (parsed, _) = Store::parse_vcard(&text);
            assert_eq!(parsed.len(), 2);
        }
        assert_eq!(Store::parse_contacts_csv(&Store::contacts_to_csv(&contacts)).len(), 2);
        let (text, lossy) = Store::decode_import_file("77u/QWRh").unwrap();
        assert_eq!((text.as_str(), lossy), ("Ada", false));
    }

    #[test]
    fn recent_messages_and_suggestions() {
        let (store, account) = store_with_account();
        let inbox = store.folder_by_role(account, FolderRole::Inbox).unwrap().unwrap().id;
        let sent = store.folder_by_role(account, FolderRole::Sent).unwrap().unwrap().id;
        let deleted = store.folder_by_role(account, FolderRole::Deleted).unwrap().unwrap().id;
        let me = EmailAddress::new("Test User", "test@axentra.example");
        let ada = EmailAddress::new("Ada", "Ada@Example.com");
        let new_person = EmailAddress::new("Newbie", "new@example.com");
        let now = Utc::now();
        let msg = |folder, from: &EmailAddress, to: Vec<EmailAddress>, mins: i64, reply: bool| {
            let mut m = Message {
                account_id: account,
                folder_id: folder,
                from: from.clone(),
                to,
                subject: format!("m{mins}"),
                received_at: now - Duration::minutes(mins),
                in_reply_to: reply.then(|| "x@y".to_string()),
                is_read: true,
                ..Default::default()
            };
            store.insert_message(&mut m, None).unwrap()
        };
        let m1 = msg(inbox, &ada, vec![me.clone()], 10, false);
        let m2 = msg(sent, &me, vec![ada.clone(), new_person.clone()], 5, false);
        msg(deleted, &ada, vec![me.clone()], 1, false);
        msg(sent, &me, vec![new_person.clone()], 3, false);
        msg(inbox, &EmailAddress::new("Bot", "noreply@service.example"), vec![me.clone()], 2, true);
        msg(inbox, &new_person, vec![me.clone()], 4, true);

        let recent = store.recent_messages_with(&["ada@example.com".into()], 5).unwrap();
        assert_eq!(recent.iter().map(|m| m.id).collect::<Vec<_>>(), vec![m2, m1]);

        add(&store, "Ada", "", "ada@example.com");
        let suggested = store.suggested_people(now - Duration::days(180), 10).unwrap();
        assert_eq!(suggested, vec![(EmailAddress::new("Newbie", "new@example.com"), 3)]);
        store.dismiss_suggested_person("NEW@example.com").unwrap();
        assert!(store.suggested_people(now - Duration::days(180), 10).unwrap().is_empty());
    }
}
