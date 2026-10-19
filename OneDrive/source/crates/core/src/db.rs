//! Local state: one row per file or folder (its inode in the mount), plus
//! pending deletes, the activity feed and small key/value settings.

use std::path::Path;

use anyhow::Result;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

pub const ROOT: u64 = 1;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    pub ino: u64,
    /// OneDrive item ID; `None` until a local creation has been uploaded.
    pub id: Option<String>,
    pub parent: u64,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// Unix milliseconds.
    pub mtime: i64,
    pub etag: Option<String>,
    pub ctag: Option<String>,
    pub qxh: Option<String>,
    pub web_url: Option<String>,
    pub shared: bool,
    /// "Always keep on this device".
    pub pinned: bool,
    /// Content is present in the local cache.
    pub cached: bool,
    /// Local content changed and must be uploaded.
    pub dirty: bool,
    /// Renamed or moved locally; the change must be sent to OneDrive.
    pub moved: bool,
    /// Never synced (ignored pattern, invalid name, or inside such a folder).
    pub local_only: bool,
    /// Last sync problem with this item, shown as a red badge.
    pub error: Option<String>,
    /// Bumped on every local content change, so an upload that raced with a
    /// write knows to go again.
    pub gen: i64,
}

impl Item {
    /// Status token exposed as the `user.axentra.onedrive.status` xattr and over D-Bus.
    pub fn status(&self) -> &'static str {
        if self.error.is_some() {
            "error"
        } else if self.local_only {
            "local-only"
        } else if self.dirty || self.moved || self.id.is_none() {
            "syncing"
        } else if self.is_dir {
            if self.pinned {
                "pinned"
            } else {
                "online"
            }
        } else if self.cached && self.pinned {
            "pinned"
        } else if self.cached {
            "local"
        } else {
            "online"
        }
    }
}

const COLS: [&str; 19] = [
    "ino",
    "id",
    "parent",
    "name",
    "is_dir",
    "size",
    "mtime",
    "etag",
    "ctag",
    "qxh",
    "web_url",
    "shared",
    "pinned",
    "cached",
    "dirty",
    "moved",
    "local_only",
    "error",
    "gen",
];

fn cols(prefix: &str) -> String {
    COLS.iter()
        .map(|c| format!("{prefix}{c}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn row_item(r: &Row) -> rusqlite::Result<Item> {
    Ok(Item {
        ino: r.get::<_, i64>(0)? as u64,
        id: r.get(1)?,
        parent: r.get::<_, i64>(2)? as u64,
        name: r.get(3)?,
        is_dir: r.get(4)?,
        size: r.get::<_, i64>(5)? as u64,
        mtime: r.get(6)?,
        etag: r.get(7)?,
        ctag: r.get(8)?,
        qxh: r.get(9)?,
        web_url: r.get(10)?,
        shared: r.get(11)?,
        pinned: r.get(12)?,
        cached: r.get(13)?,
        dirty: r.get(14)?,
        moved: r.get(15)?,
        local_only: r.get(16)?,
        error: r.get(17)?,
        gen: r.get(18)?,
    })
}

const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA foreign_keys=OFF;
CREATE TABLE IF NOT EXISTS items (
  ino INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT UNIQUE,
  parent INTEGER NOT NULL,
  name TEXT NOT NULL,
  is_dir INTEGER NOT NULL,
  size INTEGER NOT NULL DEFAULT 0,
  mtime INTEGER NOT NULL DEFAULT 0,
  etag TEXT, ctag TEXT, qxh TEXT, web_url TEXT,
  shared INTEGER NOT NULL DEFAULT 0,
  pinned INTEGER NOT NULL DEFAULT 0,
  cached INTEGER NOT NULL DEFAULT 0,
  dirty INTEGER NOT NULL DEFAULT 0,
  moved INTEGER NOT NULL DEFAULT 0,
  local_only INTEGER NOT NULL DEFAULT 0,
  error TEXT,
  gen INTEGER NOT NULL DEFAULT 0,
  seen INTEGER NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX IF NOT EXISTS items_by_name ON items(parent, name COLLATE NOCASE);
CREATE TABLE IF NOT EXISTS tombstones (id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL, is_dir INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS activity (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  ts INTEGER NOT NULL,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  folder TEXT NOT NULL,
  path TEXT NOT NULL,
  is_dir INTEGER NOT NULL,
  detail TEXT NOT NULL DEFAULT ''
);
INSERT OR IGNORE INTO items(ino, parent, name, is_dir, mtime) VALUES (1, 0, '', 1, 0);
";

/// One entry in the flyout's activity list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub seq: i64,
    /// Unix seconds.
    pub ts: i64,
    /// uploaded, downloaded, deleted, deleted_online, added_online,
    /// updated_online, renamed, moved, conflict, error
    pub kind: String,
    pub name: String,
    /// Display name of the containing folder ("OneDrive" for the root).
    pub folder: String,
    /// Path relative to the mount root, starting with '/'.
    pub path: String,
    pub is_dir: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct Tombstone {
    pub id: String,
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

pub struct Db(Mutex<Connection>);

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self(Mutex::new(conn)))
    }

    pub fn with<R>(&self, f: impl FnOnce(&Store<'_>) -> R) -> R {
        let guard = self.0.lock();
        f(&Store(&guard))
    }

    pub fn tx<R>(&self, f: impl FnOnce(&Store<'_>) -> Result<R>) -> Result<R> {
        let mut guard = self.0.lock();
        let tx = guard.transaction()?;
        let r = f(&Store(&tx))?;
        tx.commit()?;
        Ok(r)
    }
}

pub struct Store<'a>(pub &'a Connection);

impl Store<'_> {
    fn one(&self, filter: &str, p: impl rusqlite::Params) -> Result<Option<Item>> {
        let sql = format!("SELECT {} FROM items WHERE {filter}", cols(""));
        Ok(self.0.query_row(&sql, p, row_item).optional()?)
    }

    fn many(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Item>> {
        let mut st = self.0.prepare_cached(sql)?;
        let rows = st.query_map(p, row_item)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get(&self, ino: u64) -> Result<Option<Item>> {
        self.one("ino = ?1", params![ino as i64])
    }

    pub fn by_id(&self, id: &str) -> Result<Option<Item>> {
        self.one("id = ?1", params![id])
    }

    /// Case-insensitive, like OneDrive itself.
    pub fn child(&self, parent: u64, name: &str) -> Result<Option<Item>> {
        self.one(
            "parent = ?1 AND name = ?2 COLLATE NOCASE",
            params![parent as i64, name],
        )
    }

    pub fn children(&self, parent: u64) -> Result<Vec<Item>> {
        self.many(
            &format!(
                "SELECT {} FROM items WHERE parent = ?1 AND ino != 1 ORDER BY ino",
                cols("")
            ),
            params![parent as i64],
        )
    }

    pub fn child_count(&self, parent: u64) -> Result<u64> {
        let n: i64 = self.0.query_row(
            "SELECT COUNT(*) FROM items WHERE parent = ?1 AND ino != 1",
            params![parent as i64],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    /// (files directly inside, how many of them are on this device)
    pub fn dir_availability(&self, parent: u64) -> Result<(u64, u64)> {
        let (total, cached): (i64, i64) = self.0.query_row(
            "SELECT COUNT(*), COALESCE(SUM(cached), 0) FROM items WHERE parent = ?1 AND ino != 1 AND is_dir = 0",
            params![parent as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((total as u64, cached as u64))
    }

    /// The item and everything below it, parents before children.
    pub fn subtree(&self, ino: u64) -> Result<Vec<Item>> {
        self.many(
            &format!(
                "WITH RECURSIVE t(ino, depth) AS (SELECT ?1, 0 UNION ALL \
                 SELECT i.ino, t.depth + 1 FROM items i JOIN t ON i.parent = t.ino WHERE i.ino != 1) \
                 SELECT {} FROM items i JOIN t ON i.ino = t.ino ORDER BY t.depth, i.ino",
                cols("i.")
            ),
            params![ino as i64],
        )
    }

    pub fn insert(&self, it: &Item) -> Result<u64> {
        self.0.execute(
            "INSERT INTO items (id,parent,name,is_dir,size,mtime,etag,ctag,qxh,web_url,shared,pinned,cached,dirty,moved,local_only,error,gen,seen) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,1)",
            params![
                it.id, it.parent as i64, it.name, it.is_dir, it.size as i64, it.mtime, it.etag, it.ctag,
                it.qxh, it.web_url, it.shared, it.pinned, it.cached, it.dirty, it.moved, it.local_only,
                it.error, it.gen
            ],
        )?;
        Ok(self.0.last_insert_rowid() as u64)
    }

    /// Write every column of `it` back to its row.
    pub fn update(&self, it: &Item) -> Result<()> {
        self.0.execute(
            "UPDATE items SET id=?2,parent=?3,name=?4,is_dir=?5,size=?6,mtime=?7,etag=?8,ctag=?9,qxh=?10,\
             web_url=?11,shared=?12,pinned=?13,cached=?14,dirty=?15,moved=?16,local_only=?17,error=?18,gen=?19,seen=1 \
             WHERE ino=?1",
            params![
                it.ino as i64, it.id, it.parent as i64, it.name, it.is_dir, it.size as i64, it.mtime,
                it.etag, it.ctag, it.qxh, it.web_url, it.shared, it.pinned, it.cached, it.dirty, it.moved,
                it.local_only, it.error, it.gen
            ],
        )?;
        Ok(())
    }

    pub fn delete(&self, ino: u64) -> Result<()> {
        if ino != ROOT {
            self.0
                .execute("DELETE FROM items WHERE ino = ?1", params![ino as i64])?;
        }
        Ok(())
    }

    pub fn set_error(&self, ino: u64, error: Option<&str>) -> Result<()> {
        self.0.execute(
            "UPDATE items SET error = ?2 WHERE ino = ?1",
            params![ino as i64, error],
        )?;
        Ok(())
    }

    /// Path relative to the mount root, e.g. "/Documents/Budget.xlsx".
    pub fn path(&self, ino: u64) -> Result<String> {
        let mut parts = Vec::new();
        let mut cur = ino;
        while cur != ROOT {
            let Some(it) = self.get(cur)? else { break };
            parts.push(it.name);
            cur = it.parent;
        }
        parts.reverse();
        Ok(format!("/{}", parts.join("/")))
    }

    /// Display name of the folder holding `ino`, as the activity list shows it.
    pub fn folder_name(&self, parent: u64) -> Result<String> {
        if parent == ROOT {
            return Ok("OneDrive".into());
        }
        Ok(self
            .get(parent)?
            .map(|p| p.name)
            .unwrap_or_else(|| "OneDrive".into()))
    }

    pub fn resolve(&self, rel: &str) -> Result<Option<Item>> {
        let mut cur = self.get(ROOT)?;
        for part in rel.split('/').filter(|p| !p.is_empty() && *p != ".") {
            let Some(dir) = cur else { return Ok(None) };
            cur = self.child(dir.ino, part)?;
        }
        Ok(cur)
    }

    pub fn is_inside(&self, ino: u64, ancestor: u64) -> Result<bool> {
        let mut cur = ino;
        while cur != ROOT && cur != 0 {
            if cur == ancestor {
                return Ok(true);
            }
            cur = match self.get(cur)? {
                Some(it) => it.parent,
                None => return Ok(false),
            };
        }
        Ok(ancestor == ROOT)
    }

    // ----- work queues ---------------------------------------------------

    fn pending(&self, cond: &str) -> Result<Vec<Item>> {
        self.many(
            &format!(
                "SELECT {} FROM items c JOIN items p ON c.parent = p.ino \
                 WHERE c.ino != 1 AND c.local_only = 0 AND p.id IS NOT NULL AND {cond} ORDER BY c.ino",
                cols("c.")
            ),
            [],
        )
    }

    pub fn folders_to_create(&self) -> Result<Vec<Item>> {
        self.pending("c.is_dir = 1 AND c.id IS NULL")
    }

    pub fn moves(&self) -> Result<Vec<Item>> {
        self.pending("c.moved = 1 AND c.id IS NOT NULL")
    }

    pub fn uploads(&self) -> Result<Vec<Item>> {
        self.pending("c.is_dir = 0 AND c.cached = 1 AND (c.dirty = 1 OR c.id IS NULL)")
    }

    pub fn pinned_uncached(&self) -> Result<Vec<Item>> {
        self.many(
            &format!(
                "SELECT {} FROM items WHERE is_dir = 0 AND pinned = 1 AND cached = 0 AND id IS NOT NULL",
                cols("")
            ),
            [],
        )
    }

    pub fn pending_count(&self) -> Result<u64> {
        let n: i64 = self.0.query_row(
            "SELECT (SELECT COUNT(*) FROM items WHERE ino != 1 AND local_only = 0 AND \
               (dirty = 1 OR moved = 1 OR id IS NULL)) + (SELECT COUNT(*) FROM tombstones)",
            [],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    pub fn errors(&self) -> Result<Vec<Item>> {
        self.many(
            &format!(
                "SELECT {} FROM items WHERE error IS NOT NULL ORDER BY ino",
                cols("")
            ),
            [],
        )
    }

    pub fn cached_bytes(&self) -> Result<u64> {
        let n: i64 = self.0.query_row(
            "SELECT COALESCE(SUM(size), 0) FROM items WHERE cached = 1 AND is_dir = 0",
            [],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    // ----- full-resync bookkeeping ----------------------------------------

    pub fn mark_all_unseen(&self) -> Result<()> {
        self.0.execute(
            "UPDATE items SET seen = 0 WHERE id IS NOT NULL AND ino != 1",
            [],
        )?;
        Ok(())
    }

    /// Synced items that a full enumeration did not mention, i.e. deleted
    /// online while we weren't watching.
    pub fn unseen(&self) -> Result<Vec<Item>> {
        self.many(
            &format!(
                "SELECT {} FROM items WHERE seen = 0 AND id IS NOT NULL AND ino != 1 ORDER BY ino DESC",
                cols("")
            ),
            [],
        )
    }

    // ----- tombstones -------------------------------------------------------

    pub fn add_tombstone(&self, t: &Tombstone) -> Result<()> {
        self.0.execute(
            "INSERT OR REPLACE INTO tombstones (id, name, path, is_dir) VALUES (?1, ?2, ?3, ?4)",
            params![t.id, t.name, t.path, t.is_dir],
        )?;
        Ok(())
    }

    pub fn tombstones(&self) -> Result<Vec<Tombstone>> {
        let mut st = self
            .0
            .prepare_cached("SELECT id, name, path, is_dir FROM tombstones ORDER BY rowid")?;
        let rows = st.query_map([], |r| {
            Ok(Tombstone {
                id: r.get(0)?,
                name: r.get(1)?,
                path: r.get(2)?,
                is_dir: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn remove_tombstone(&self, id: &str) -> Result<()> {
        self.0
            .execute("DELETE FROM tombstones WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ----- key/value ----------------------------------------------------------

    pub fn kv(&self, k: &str) -> Result<Option<String>> {
        Ok(self
            .0
            .query_row("SELECT v FROM kv WHERE k = ?1", params![k], |r| r.get(0))
            .optional()?)
    }

    pub fn set_kv(&self, k: &str, v: &str) -> Result<()> {
        self.0.execute(
            "INSERT INTO kv (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![k, v],
        )?;
        Ok(())
    }

    pub fn del_kv(&self, k: &str) -> Result<()> {
        self.0.execute("DELETE FROM kv WHERE k = ?1", params![k])?;
        Ok(())
    }

    // ----- activity -----------------------------------------------------------

    pub fn add_activity(&self, kind: &str, item: &Item, detail: &str) -> Result<()> {
        let path = self.path(item.ino)?;
        let folder = self.folder_name(item.parent)?;
        self.add_activity_raw(kind, &item.name, &folder, &path, item.is_dir, detail)
    }

    pub fn add_activity_raw(
        &self,
        kind: &str,
        name: &str,
        folder: &str,
        path: &str,
        is_dir: bool,
        detail: &str,
    ) -> Result<()> {
        self.0.execute(
            "INSERT INTO activity (ts, kind, name, folder, path, is_dir, detail) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![chrono::Utc::now().timestamp(), kind, name, folder, path, is_dir, detail],
        )?;
        // Keep the feed bounded.
        self.0.execute(
            "DELETE FROM activity WHERE seq <= (SELECT MAX(seq) FROM activity) - 500",
            [],
        )?;
        Ok(())
    }

    pub fn activity(&self, limit: u32) -> Result<Vec<Activity>> {
        let mut st = self.0.prepare_cached(
            "SELECT seq, ts, kind, name, folder, path, is_dir, detail FROM activity ORDER BY seq DESC LIMIT ?1",
        )?;
        let rows = st.query_map(params![limit], |r| {
            Ok(Activity {
                seq: r.get(0)?,
                ts: r.get(1)?,
                kind: r.get(2)?,
                name: r.get(3)?,
                folder: r.get(4)?,
                path: r.get(5)?,
                is_dir: r.get(6)?,
                detail: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Forget everything (after "Unlink this PC").
    pub fn reset(&self) -> Result<()> {
        self.0.execute_batch(
            "DELETE FROM items WHERE ino != 1; DELETE FROM tombstones; DELETE FROM kv; DELETE FROM activity;\
             UPDATE items SET id = NULL, etag = NULL, ctag = NULL WHERE ino = 1;",
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(parent: u64, name: &str) -> Item {
        Item {
            parent,
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn names_are_case_insensitive_and_unique() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.sqlite")).unwrap();
        db.with(|s| {
            let a = s.insert(&file(ROOT, "Report.docx")).unwrap();
            assert_eq!(s.child(ROOT, "report.DOCX").unwrap().unwrap().ino, a);
            assert!(s.insert(&file(ROOT, "REPORT.docx")).is_err());
        });
    }

    #[test]
    fn paths_subtrees_and_queues() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.sqlite")).unwrap();
        db.with(|s| {
            let mut root = s.get(ROOT).unwrap().unwrap();
            root.id = Some("ROOT".into());
            s.update(&root).unwrap();
            let docs = s
                .insert(&Item {
                    is_dir: true,
                    ..file(ROOT, "Documents")
                })
                .unwrap();
            let f = s
                .insert(&Item {
                    cached: true,
                    dirty: true,
                    ..file(docs, "a.txt")
                })
                .unwrap();
            assert_eq!(s.path(f).unwrap(), "/Documents/a.txt");
            assert_eq!(s.resolve("/Documents/A.TXT").unwrap().unwrap().ino, f);
            assert_eq!(
                s.subtree(docs)
                    .unwrap()
                    .iter()
                    .map(|i| i.ino)
                    .collect::<Vec<_>>(),
                vec![docs, f]
            );
            // The folder has to exist online before its file can upload.
            assert_eq!(s.folders_to_create().unwrap().len(), 1);
            assert!(s.uploads().unwrap().is_empty());
            let mut d = s.get(docs).unwrap().unwrap();
            d.id = Some("DOCS".into());
            s.update(&d).unwrap();
            assert_eq!(s.uploads().unwrap()[0].ino, f);
            assert!(s.is_inside(f, docs).unwrap());
            assert!(!s.is_inside(docs, f).unwrap());
        });
    }
}
