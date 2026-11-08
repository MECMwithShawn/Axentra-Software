//! Organization: archiving ("tombstoning") accounts you no longer use, previous passwords from
//! entry history, and password health checks.
//!
//! An archived entry keeps its place and data but drops out of search and normal lists. It is
//! marked with the tag `Archived` (visible in KeePass too) and the archive date in custom data.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use keepass::Database;
use keepass::db::{
    CustomDataItem, CustomDataValue, Entry, EntryId, EntryMut, EntryRef, History, Times,
};

pub const ARCHIVE_TAG: &str = "Archived";
pub const ARCHIVED_AT_KEY: &str = "AxentraVault.ArchivedAt";

/// Whether the entry sits anywhere inside the recycle bin.
pub fn in_recycle_bin(e: &EntryRef<'_>) -> bool {
    let db = e.database();
    let Some(bin) = db.recycle_bin().map(|g| g.id()) else {
        return false;
    };
    let mut id = Some(e.parent().id());
    while let Some(gid) = id {
        if gid == bin {
            return true;
        }
        id = db.group(gid).and_then(|g| g.parent().map(|p| p.id()));
    }
    false
}

/// Favourites are a "Favorite" tag, so KeePass and KeePassXC show and keep them too.
pub const FAVORITE_TAG: &str = "Favorite";

pub fn is_favorite(e: &Entry) -> bool {
    e.tags
        .iter()
        .any(|t| t.eq_ignore_ascii_case(FAVORITE_TAG) || t.eq_ignore_ascii_case("Favourite"))
}

pub fn set_favorite(e: &mut Entry, on: bool) {
    e.tags
        .retain(|t| !(t.eq_ignore_ascii_case(FAVORITE_TAG) || t.eq_ignore_ascii_case("Favourite")));
    if on {
        e.tags.push(FAVORITE_TAG.to_owned());
    }
}

/// Tags the user sees: everything except the app's own markers.
pub fn user_tags(e: &Entry) -> impl Iterator<Item = &String> {
    e.tags.iter().filter(|t| {
        !t.eq_ignore_ascii_case(ARCHIVE_TAG)
            && !t.eq_ignore_ascii_case(FAVORITE_TAG)
            && !t.eq_ignore_ascii_case("Favourite")
    })
}

/// Every user tag in the database with how many active entries carry it, most used first.
pub fn tag_counts(db: &Database) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, (String, usize)> = HashMap::new();
    for e in db
        .iter_all_entries()
        .filter(|e| !is_archived(e) && !in_recycle_bin(e))
    {
        for t in user_tags(&e) {
            let slot = counts
                .entry(t.to_lowercase())
                .or_insert_with(|| (t.clone(), 0));
            slot.1 += 1;
        }
    }
    let mut v: Vec<(String, usize)> = counts.into_values().collect();
    v.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    v
}

pub fn is_archived(e: &Entry) -> bool {
    e.tags.iter().any(|t| t.eq_ignore_ascii_case(ARCHIVE_TAG))
}

pub fn archived_at(e: &Entry) -> Option<NaiveDateTime> {
    e.custom_data
        .get(ARCHIVED_AT_KEY)
        .and_then(|i| i.last_modification_time)
}

pub fn set_archived(e: &mut Entry, archived: bool) {
    e.tags.retain(|t| !t.eq_ignore_ascii_case(ARCHIVE_TAG));
    e.custom_data.remove(ARCHIVED_AT_KEY);
    if archived {
        let now = Times::now();
        e.tags.push(ARCHIVE_TAG.to_owned());
        e.custom_data.insert(
            ARCHIVED_AT_KEY.to_owned(),
            CustomDataItem {
                value: Some(CustomDataValue::String(
                    now.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                )),
                last_modification_time: Some(now),
            },
        );
    }
    e.times.last_modification = Some(Times::now());
}

/// KeePass's default when the database does not set a limit.
const DEFAULT_HISTORY_MAX_ITEMS: usize = 10;

/// Applies `edit` to an entry, keeping the previous version in its history the way KeePass does:
/// oldest first, newest appended last, trimmed to the database's history limit from the oldest
/// end. (The `keepass` crate's own change tracking inserts at the front, which KeePass would read
/// as the oldest version and trim first.) Returns false if the entry does not exist.
pub fn edit_with_history(
    db: &mut Database,
    id: EntryId,
    edit: impl FnOnce(&mut EntryMut<'_>),
) -> bool {
    let limit = match db.meta.history_max_items {
        Some(n) if n < 0 => usize::MAX,
        Some(n) => n as usize,
        None => DEFAULT_HISTORY_MAX_ITEMS,
    };
    let Some(mut e) = db.entry_mut(id) else {
        return false;
    };

    let mut snapshot: Entry = (*e).clone();
    snapshot.history = None;
    let mut versions: Vec<Entry> = e
        .history
        .as_ref()
        .map(|h| h.get_entries().clone())
        .unwrap_or_default();

    edit(&mut e);

    versions.push(snapshot);
    let excess = versions.len().saturating_sub(limit);
    versions.drain(..excess);
    let mut history = History::default();
    // add_entry inserts at the front, so add newest first to end up oldest first.
    for v in versions.into_iter().rev() {
        history.add_entry(v);
    }
    e.history = (!history.get_entries().is_empty()).then_some(history);
    e.times.last_modification = Some(Times::now());
    true
}

/// A password this entry used before, and when it stopped being current.
pub struct PreviousPassword {
    pub password: String,
    pub replaced: Option<NaiveDateTime>,
}

/// Distinct earlier passwords from history, newest first, excluding the current one.
pub fn previous_passwords(e: &Entry) -> Vec<PreviousPassword> {
    let current = e.get_password().unwrap_or("");
    let Some(history) = &e.history else {
        return Vec::new();
    };
    let versions = history.get_entries();

    let mut out: Vec<PreviousPassword> = Vec::new();
    // History is oldest first. A password was replaced when the next version (or the current
    // entry) stopped using it.
    for (i, v) in versions.iter().enumerate() {
        let pw = v.get_password().unwrap_or("");
        if pw.is_empty() || pw == current {
            continue;
        }
        let next_changed = versions[i + 1..]
            .iter()
            .find(|n| n.get_password().unwrap_or("") != pw);
        let replaced = match next_changed {
            Some(n) => n.times.last_modification,
            None => e.times.last_modification,
        };
        out.retain(|p| p.password != pw);
        out.push(PreviousPassword {
            password: pw.to_owned(),
            replaced,
        });
    }
    out.reverse();
    out
}

/// When the current password was set: the oldest unbroken run of history using it.
pub fn password_set_at(e: &Entry) -> Option<NaiveDateTime> {
    let current = e.get_password().unwrap_or("");
    let mut since = e.times.last_modification.or(e.times.creation);
    if let Some(h) = &e.history {
        for v in h.get_entries().iter().rev() {
            if v.get_password().unwrap_or("") != current {
                break;
            }
            since = v.times.last_modification.or(v.times.creation).or(since);
        }
    }
    since
}

/// Rough strength in bits: length times log2 of the character classes used.
pub fn estimate_bits(password: &str) -> f64 {
    let (mut lower, mut upper, mut digit, mut other) = (false, false, false, false);
    for c in password.chars() {
        match c {
            'a'..='z' => lower = true,
            'A'..='Z' => upper = true,
            '0'..='9' => digit = true,
            _ => other = true,
        }
    }
    let pool = 26 * lower as u32 + 26 * upper as u32 + 10 * digit as u32 + 33 * other as u32;
    if pool < 2 {
        0.0
    } else {
        password.chars().count() as f64 * (pool as f64).log2()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Issue {
    Reused,
    Weak,
    Old,
    /// Found in a known data breach (the app's opt-in online check; `health` never sets it).
    Breached,
}

impl Issue {
    pub fn label(self) -> &'static str {
        match self {
            Issue::Reused => "reused",
            Issue::Weak => "weak",
            Issue::Old => "over a year old",
            Issue::Breached => "found in a data breach",
        }
    }
}

pub const WEAK_BITS: f64 = 50.0;
pub const OLD_DAYS: i64 = 365;

/// Password problems for every active (not archived, not recycled) entry with a password.
pub fn health(db: &Database) -> HashMap<EntryId, Vec<Issue>> {
    let active: Vec<_> = db
        .iter_all_entries()
        .filter(|e| {
            !is_archived(e) && !in_recycle_bin(e) && !e.get_password().unwrap_or("").is_empty()
        })
        .collect();

    let mut uses: HashMap<&str, usize> = HashMap::new();
    for e in &active {
        *uses.entry(e.get_password().unwrap_or("")).or_default() += 1;
    }

    let now = Times::now();
    let mut out = HashMap::new();
    for e in &active {
        let pw = e.get_password().unwrap_or("");
        let mut issues = Vec::new();
        if uses[pw] > 1 {
            issues.push(Issue::Reused);
        }
        if estimate_bits(pw) < WEAK_BITS {
            issues.push(Issue::Weak);
        }
        if password_set_at(e).is_some_and(|t| (now - t).num_days() > OLD_DAYS) {
            issues.push(Issue::Old);
        }
        if !issues.is_empty() {
            out.insert(e.id(), issues);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use keepass::db::fields;

    use super::*;

    #[test]
    fn favorites_and_tags() {
        let mut db = Database::new();
        let a = db.root_mut().add_entry().id();
        let b = db.root_mut().add_entry().id();
        set_favorite(&mut db.entry_mut(a).unwrap(), true);
        db.entry_mut(a).unwrap().tags.push("Work".into());
        db.entry_mut(b).unwrap().tags.push("work".into());
        set_archived(&mut db.entry_mut(b).unwrap(), false);
        assert!(is_favorite(&db.entry(a).unwrap()));
        let tags = tag_counts(&db);
        assert_eq!(
            tags.len(),
            1,
            "favourite/archive markers are not user tags; case-insensitive merge"
        );
        assert_eq!(tags[0].1, 2);
        set_favorite(&mut db.entry_mut(a).unwrap(), false);
        assert!(!is_favorite(&db.entry(a).unwrap()));
    }

    #[test]
    fn archive_round_trip() {
        let mut db = Database::new();
        let id = db.root_mut().add_entry().id();
        let mut e = db.entry_mut(id).unwrap();
        e.tags.push("work".into());
        set_archived(&mut e, true);
        assert!(is_archived(&e));
        assert!(archived_at(&e).is_some());
        assert!(e.tags.contains(&"work".to_owned()));
        set_archived(&mut e, false);
        assert!(!is_archived(&e));
        assert!(archived_at(&e).is_none());
        assert_eq!(e.tags, vec!["work".to_owned()]);
    }

    #[test]
    fn previous_passwords_from_history() {
        let mut db = Database::new();
        let id = db.root_mut().add_entry().id();
        for pw in ["first", "second", "second", "third"] {
            edit_with_history(&mut db, id, |e| e.set_protected(fields::PASSWORD, pw));
        }
        let e = db.entry(id).unwrap();
        let prev: Vec<String> = previous_passwords(&e)
            .into_iter()
            .map(|p| p.password)
            .collect();
        assert_eq!(prev, vec!["second".to_owned(), "first".to_owned()]);
    }

    #[test]
    fn history_is_oldest_first_and_trimmed() {
        let mut db = Database::new();
        db.meta.history_max_items = Some(3);
        let id = db
            .root_mut()
            .add_entry()
            .edit(|e| e.set_protected(fields::PASSWORD, "p0"))
            .id();
        for i in 1..=5 {
            edit_with_history(&mut db, id, |e| {
                e.set_protected(fields::PASSWORD, format!("p{i}"))
            });
        }
        let e = db.entry(id).unwrap();
        let order: Vec<&str> = e
            .history
            .as_ref()
            .unwrap()
            .get_entries()
            .iter()
            .map(|v| v.get_password().unwrap())
            .collect();
        assert_eq!(order, vec!["p2", "p3", "p4"]);
        assert_eq!(e.get_password(), Some("p5"));
    }

    #[test]
    fn health_flags_reused_and_weak() {
        let mut db = Database::new();
        let mut root = db.root_mut();
        let a = root
            .add_entry()
            .edit(|e| e.set_protected(fields::PASSWORD, "hunter2"))
            .id();
        let b = root
            .add_entry()
            .edit(|e| e.set_protected(fields::PASSWORD, "hunter2"))
            .id();
        let c = root
            .add_entry()
            .edit(|e| e.set_protected(fields::PASSWORD, "Xq9#vL2!mR7@pT4$wZ8&"))
            .id();
        let h = health(&db);
        assert!(h[&a].contains(&Issue::Reused) && h[&a].contains(&Issue::Weak));
        assert!(h[&b].contains(&Issue::Reused));
        assert!(!h.contains_key(&c));

        // Archiving one side clears the reuse flag on the other.
        set_archived(&mut db.entry_mut(b).unwrap(), true);
        assert!(!health(&db)[&a].contains(&Issue::Reused));
    }

    #[test]
    fn strength_estimate() {
        assert!(estimate_bits("password") < WEAK_BITS);
        assert!(estimate_bits("Xq9#vL2!mR7@pT4$wZ8&") > 100.0);
    }
}
