//! The mirror's own record of every source item it has seen: where it went,
//! which version, how far that was verified, and what went wrong.
//!
//! A separate SQLite file (`mirror.sqlite`) with its own connection. The
//! sync engine's `state.sqlite` is never opened here, and nothing that
//! resets the engine's state touches this file, so it outlives an unlink.
//!
//! The schema is versioned with `PRAGMA user_version`. A file from a newer
//! version is refused before anything is written to it.

use std::collections::HashMap;
use std::fmt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::str::FromStr;

use axentra_clouddrive_provider::{ItemKind, VerifyLevel};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::error::{MResult, MirrorError};

/// The schema version this code writes and understands.
pub const MANIFEST_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS mirror_items (
  source_id TEXT PRIMARY KEY,
  source_path TEXT NOT NULL,
  kind TEXT NOT NULL,
  size INTEGER NOT NULL DEFAULT 0,
  qxh TEXT,
  ctag TEXT,
  mtime TEXT,
  sha1 TEXT,
  sha256 TEXT,
  dest_path TEXT,
  dest_uid TEXT,
  dest_claimed_sha1 TEXT,
  state TEXT NOT NULL,
  verify_level INTEGER NOT NULL DEFAULT 0,
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at INTEGER,
  last_error TEXT,
  first_mirrored_at TEXT,
  last_verified_at TEXT,
  upload_pending INTEGER NOT NULL DEFAULT 0,
  adopted INTEGER NOT NULL DEFAULT 0,
  detail TEXT
);
CREATE INDEX IF NOT EXISTS mirror_items_by_state ON mirror_items(state);
CREATE TABLE IF NOT EXISTS mirror_runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  mode TEXT NOT NULL,
  started TEXT NOT NULL,
  finished TEXT,
  outcome TEXT,
  folders_created INTEGER NOT NULL DEFAULT 0,
  files_mirrored INTEGER NOT NULL DEFAULT 0,
  files_adopted INTEGER NOT NULL DEFAULT 0,
  files_diverged INTEGER NOT NULL DEFAULT 0,
  files_skipped INTEGER NOT NULL DEFAULT 0,
  files_failed INTEGER NOT NULL DEFAULT 0,
  bytes_uploaded INTEGER NOT NULL DEFAULT 0,
  staging_peak INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS mirror_kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);
";

/// Where an item stands. Nothing ever moves an item out of the source or
/// the destination; these only describe it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    /// Known, nothing transferred.
    Planned,
    /// Downloaded and checked against the source ([`VerifyLevel::L1SourceVerified`]).
    Staged,
    /// The destination accepted it ([`VerifyLevel::L2Uploaded`]) and no
    /// further level has been reached yet.
    Uploaded,
    /// As verified as this run was asked to get it.
    Verified,
    /// Deliberately not mirrored (too large, not plain content).
    Skipped,
    /// Source and destination disagree. Both are left exactly as they are.
    Diverged,
    /// The last attempt failed; see `last_error` and `next_attempt_at`.
    Failed,
    /// Gone from the source. Whatever the destination holds is kept.
    SourceDeleted,
}

impl ItemState {
    pub const ALL: [ItemState; 8] = [
        ItemState::Planned,
        ItemState::Staged,
        ItemState::Uploaded,
        ItemState::Verified,
        ItemState::Skipped,
        ItemState::Diverged,
        ItemState::Failed,
        ItemState::SourceDeleted,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ItemState::Planned => "planned",
            ItemState::Staged => "staged",
            ItemState::Uploaded => "uploaded",
            ItemState::Verified => "verified",
            ItemState::Skipped => "skipped",
            ItemState::Diverged => "diverged",
            ItemState::Failed => "failed",
            ItemState::SourceDeleted => "source_deleted",
        }
    }
}

impl fmt::Display for ItemState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ItemState {
    type Err = MirrorError;

    fn from_str(s: &str) -> MResult<Self> {
        ItemState::ALL
            .into_iter()
            .find(|st| st.as_str() == s)
            .ok_or_else(|| MirrorError::Corrupt(format!("unknown item state {s:?}")))
    }
}

pub(crate) fn level_to_i64(l: VerifyLevel) -> i64 {
    match l {
        VerifyLevel::L0Planned => 0,
        VerifyLevel::L1SourceVerified => 1,
        VerifyLevel::L2Uploaded => 2,
        VerifyLevel::L3RemoteClaimed => 3,
        VerifyLevel::L4RoundTrip => 4,
    }
}

pub(crate) fn level_from_i64(n: i64) -> MResult<VerifyLevel> {
    Ok(match n {
        0 => VerifyLevel::L0Planned,
        1 => VerifyLevel::L1SourceVerified,
        2 => VerifyLevel::L2Uploaded,
        3 => VerifyLevel::L3RemoteClaimed,
        4 => VerifyLevel::L4RoundTrip,
        _ => return Err(MirrorError::Corrupt(format!("unknown verify level {n}"))),
    })
}

pub(crate) fn kind_str(k: ItemKind) -> &'static str {
    match k {
        ItemKind::File => "file",
        ItemKind::Folder => "folder",
    }
}

fn kind_from_str(s: &str) -> MResult<ItemKind> {
    match s {
        "file" => Ok(ItemKind::File),
        "folder" => Ok(ItemKind::Folder),
        _ => Err(MirrorError::Corrupt(format!("unknown item kind {s:?}"))),
    }
}

/// One row of `mirror_items`.
///
/// `size`, `qxh`, `ctag` and `mtime` describe the source version this row
/// is about (the one mirrored, once anything reached the destination).
/// `sha1` and `sha256` are only ever *computed* here from staged bytes,
/// never copied from a provider's claim, so they are empty until
/// [`VerifyLevel::L1SourceVerified`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    pub source_id: String,
    /// Relative to the source's root, `/`-separated.
    pub source_path: String,
    pub kind: ItemKind,
    pub size: u64,
    pub qxh: Option<String>,
    pub ctag: Option<String>,
    /// RFC 3339.
    pub mtime: Option<String>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
    /// Relative to the destination's root, `/`-separated.
    pub dest_path: Option<String>,
    pub dest_uid: Option<String>,
    pub dest_claimed_sha1: Option<String>,
    pub state: ItemState,
    pub verify_level: VerifyLevel,
    /// Failed attempts in a row; reset by a success.
    pub attempts: u32,
    /// Unix seconds before which the item is not tried again.
    pub next_attempt_at: Option<i64>,
    pub last_error: Option<String>,
    /// RFC 3339.
    pub first_mirrored_at: Option<String>,
    /// RFC 3339.
    pub last_verified_at: Option<String>,
    /// An upload was started and its outcome is not known: before trying
    /// again, look at the destination first.
    pub upload_pending: bool,
    /// Found already at the destination with the same content, rather than
    /// uploaded by the mirror.
    pub adopted: bool,
    /// Why it was skipped or diverged, or what was noticed about it.
    pub detail: Option<String>,
}

impl ItemRow {
    pub fn new(source_id: &str, source_path: &str, kind: ItemKind) -> Self {
        Self {
            source_id: source_id.into(),
            source_path: source_path.into(),
            kind,
            size: 0,
            qxh: None,
            ctag: None,
            mtime: None,
            sha1: None,
            sha256: None,
            dest_path: None,
            dest_uid: None,
            dest_claimed_sha1: None,
            state: ItemState::Planned,
            verify_level: VerifyLevel::L0Planned,
            attempts: 0,
            next_attempt_at: None,
            last_error: None,
            first_mirrored_at: None,
            last_verified_at: None,
            upload_pending: false,
            adopted: false,
            detail: None,
        }
    }

    /// Something of this item may be at the destination because of the
    /// mirror: it must not be uploaded as new again without looking.
    pub fn reached_destination(&self) -> bool {
        match self.kind {
            ItemKind::Folder => {
                matches!(self.state, ItemState::Verified) || self.dest_uid.is_some()
            }
            ItemKind::File => {
                self.verify_level >= VerifyLevel::L2Uploaded
                    || self.upload_pending
                    || self.adopted
                    || self.dest_uid.is_some()
            }
        }
    }

    fn from_row(r: &Row) -> MResult<Self> {
        let kind: String = r.get("kind")?;
        let state: String = r.get("state")?;
        Ok(Self {
            source_id: r.get("source_id")?,
            source_path: r.get("source_path")?,
            kind: kind_from_str(&kind)?,
            size: r.get::<_, i64>("size")?.max(0) as u64,
            qxh: r.get("qxh")?,
            ctag: r.get("ctag")?,
            mtime: r.get("mtime")?,
            sha1: r.get("sha1")?,
            sha256: r.get("sha256")?,
            dest_path: r.get("dest_path")?,
            dest_uid: r.get("dest_uid")?,
            dest_claimed_sha1: r.get("dest_claimed_sha1")?,
            state: state.parse()?,
            verify_level: level_from_i64(r.get("verify_level")?)?,
            attempts: r.get::<_, i64>("attempts")?.clamp(0, u32::MAX as i64) as u32,
            next_attempt_at: r.get("next_attempt_at")?,
            last_error: r.get("last_error")?,
            first_mirrored_at: r.get("first_mirrored_at")?,
            last_verified_at: r.get("last_verified_at")?,
            upload_pending: r.get("upload_pending")?,
            adopted: r.get("adopted")?,
            detail: r.get("detail")?,
        })
    }
}

/// One row of `mirror_runs`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRow {
    pub id: i64,
    /// `dry` or `real`.
    pub mode: String,
    pub started: String,
    pub finished: Option<String>,
    pub outcome: Option<String>,
    pub folders_created: u64,
    pub files_mirrored: u64,
    pub files_adopted: u64,
    pub files_diverged: u64,
    pub files_skipped: u64,
    pub files_failed: u64,
    pub bytes_uploaded: u64,
    pub staging_peak: u64,
}

pub struct Manifest {
    conn: Connection,
}

const UPSERT: &str = "
INSERT INTO mirror_items (
  source_id, source_path, kind, size, qxh, ctag, mtime, sha1, sha256,
  dest_path, dest_uid, dest_claimed_sha1, state, verify_level, attempts,
  next_attempt_at, last_error, first_mirrored_at, last_verified_at,
  upload_pending, adopted, detail
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
          ?16, ?17, ?18, ?19, ?20, ?21, ?22)
ON CONFLICT(source_id) DO UPDATE SET
  source_path = excluded.source_path, kind = excluded.kind, size = excluded.size,
  qxh = excluded.qxh, ctag = excluded.ctag, mtime = excluded.mtime,
  sha1 = excluded.sha1, sha256 = excluded.sha256, dest_path = excluded.dest_path,
  dest_uid = excluded.dest_uid, dest_claimed_sha1 = excluded.dest_claimed_sha1,
  state = excluded.state, verify_level = excluded.verify_level,
  attempts = excluded.attempts, next_attempt_at = excluded.next_attempt_at,
  last_error = excluded.last_error, first_mirrored_at = excluded.first_mirrored_at,
  last_verified_at = excluded.last_verified_at,
  upload_pending = excluded.upload_pending, adopted = excluded.adopted,
  detail = excluded.detail
";

fn put_row(conn: &Connection, r: &ItemRow) -> rusqlite::Result<()> {
    conn.execute(
        UPSERT,
        params![
            r.source_id,
            r.source_path,
            kind_str(r.kind),
            r.size as i64,
            r.qxh,
            r.ctag,
            r.mtime,
            r.sha1,
            r.sha256,
            r.dest_path,
            r.dest_uid,
            r.dest_claimed_sha1,
            r.state.as_str(),
            level_to_i64(r.verify_level),
            r.attempts as i64,
            r.next_attempt_at,
            r.last_error,
            r.first_mirrored_at,
            r.last_verified_at,
            r.upload_pending,
            r.adopted,
            r.detail,
        ],
    )?;
    Ok(())
}

impl Manifest {
    /// Open the manifest at `path`, creating it (mode 0600) and its schema
    /// if needed. Opening it again is harmless. A manifest from a newer
    /// version is refused and left unchanged.
    pub fn open(path: &Path) -> MResult<Self> {
        // Created private before SQLite opens it, so it never exists with
        // wider permissions.
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)?;
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::init(conn)
    }

    /// A manifest that lives only as long as this value, for tests and dry
    /// runs that keep nothing.
    pub fn open_in_memory() -> MResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> MResult<Self> {
        // Checked before anything is written, so a newer file stays as it is.
        let found: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if found > MANIFEST_VERSION {
            return Err(MirrorError::NewerManifest {
                found,
                supported: MANIFEST_VERSION,
            });
        }
        let tx = conn.transaction()?;
        tx.execute_batch(SCHEMA)?;
        tx.execute_batch(&format!("PRAGMA user_version = {MANIFEST_VERSION}"))?;
        tx.commit()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
        Ok(Self { conn })
    }

    pub fn user_version(&self) -> MResult<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    pub fn get(&self, source_id: &str) -> MResult<Option<ItemRow>> {
        let mut st = self
            .conn
            .prepare_cached("SELECT * FROM mirror_items WHERE source_id = ?1")?;
        let mut rows = st.query([source_id])?;
        match rows.next()? {
            Some(r) => Ok(Some(ItemRow::from_row(r)?)),
            None => Ok(None),
        }
    }

    /// Every item, ordered by source path.
    pub fn items(&self) -> MResult<Vec<ItemRow>> {
        let mut st = self
            .conn
            .prepare_cached("SELECT * FROM mirror_items ORDER BY source_path, source_id")?;
        let mut rows = st.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(ItemRow::from_row(r)?);
        }
        Ok(out)
    }

    /// Every item by source id: a plain value the planner can hold across
    /// awaits.
    pub fn snapshot(&self) -> MResult<HashMap<String, ItemRow>> {
        Ok(self
            .items()?
            .into_iter()
            .map(|r| (r.source_id.clone(), r))
            .collect())
    }

    /// Write one row, all of it, in its own transaction.
    pub fn put(&mut self, row: &ItemRow) -> MResult<()> {
        self.put_many(std::slice::from_ref(row))
    }

    /// Write several rows, all or none.
    pub fn put_many(&mut self, rows: &[ItemRow]) -> MResult<()> {
        let tx = self.conn.transaction()?;
        for r in rows {
            put_row(&tx, r)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn kv_get(&self, key: &str) -> MResult<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT v FROM mirror_kv WHERE k = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn kv_set(&mut self, key: &str, value: &str) -> MResult<()> {
        self.conn.execute(
            "INSERT INTO mirror_kv (k, v) VALUES (?1, ?2) \
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![key, value],
        )?;
        Ok(())
    }

    /// Start a run of `mode` (`dry` or `real`). Returns its id.
    pub fn begin_run(&mut self, mode: &str, started: &str) -> MResult<i64> {
        self.conn.execute(
            "INSERT INTO mirror_runs (mode, started) VALUES (?1, ?2)",
            params![mode, started],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Close run `run.id` with its outcome and totals.
    pub fn finish_run(&mut self, run: &RunRow) -> MResult<()> {
        let n = self.conn.execute(
            "UPDATE mirror_runs SET finished = ?2, outcome = ?3, folders_created = ?4, \
             files_mirrored = ?5, files_adopted = ?6, files_diverged = ?7, files_skipped = ?8, \
             files_failed = ?9, bytes_uploaded = ?10, staging_peak = ?11 WHERE id = ?1",
            params![
                run.id,
                run.finished,
                run.outcome,
                run.folders_created as i64,
                run.files_mirrored as i64,
                run.files_adopted as i64,
                run.files_diverged as i64,
                run.files_skipped as i64,
                run.files_failed as i64,
                run.bytes_uploaded as i64,
                run.staging_peak as i64,
            ],
        )?;
        if n != 1 {
            return Err(MirrorError::Corrupt(format!("no run {}", run.id)));
        }
        Ok(())
    }

    /// The most recent runs, newest first.
    pub fn runs(&self, limit: usize) -> MResult<Vec<RunRow>> {
        let mut st = self.conn.prepare_cached(
            "SELECT id, mode, started, finished, outcome, folders_created, files_mirrored, \
             files_adopted, files_diverged, files_skipped, files_failed, bytes_uploaded, \
             staging_peak FROM mirror_runs ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit as i64], |r| {
            let n = |i: usize| r.get::<_, i64>(i).map(|v| v.max(0) as u64);
            Ok(RunRow {
                id: r.get(0)?,
                mode: r.get(1)?,
                started: r.get(2)?,
                finished: r.get(3)?,
                outcome: r.get(4)?,
                folders_created: n(5)?,
                files_mirrored: n(6)?,
                files_adopted: n(7)?,
                files_diverged: n(8)?,
                files_skipped: n(9)?,
                files_failed: n(10)?,
                bytes_uploaded: n(11)?,
                staging_peak: n(12)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ItemRow {
        let mut r = ItemRow::new("A!1", "Docs/файл 📄.txt", ItemKind::File);
        r.size = 42;
        r.qxh = Some("q".into());
        r.sha256 = Some("ab".into());
        r.state = ItemState::Uploaded;
        r.verify_level = VerifyLevel::L2Uploaded;
        r.attempts = 3;
        r.next_attempt_at = Some(1_900_000_000);
        r.last_error = Some("temporary failure: 503".into());
        r.upload_pending = true;
        r
    }

    #[test]
    fn schema_is_created_once_and_reopening_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mirror.sqlite");
        {
            let mut m = Manifest::open(&path).unwrap();
            assert_eq!(m.user_version().unwrap(), MANIFEST_VERSION);
            m.put(&sample()).unwrap();
            m.kv_set("dest_root", "OneDrive").unwrap();
        }
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        for _ in 0..2 {
            let m = Manifest::open(&path).unwrap();
            assert_eq!(m.user_version().unwrap(), MANIFEST_VERSION);
            assert_eq!(m.items().unwrap(), vec![sample()]);
            assert_eq!(m.kv_get("dest_root").unwrap().as_deref(), Some("OneDrive"));
        }
    }

    #[test]
    fn a_newer_manifest_is_refused_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mirror.sqlite");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE future (x); PRAGMA user_version = 2;")
                .unwrap();
        }
        let err = Manifest::open(&path).err().expect("refused");
        assert!(
            matches!(
                err,
                MirrorError::NewerManifest {
                    found: 2,
                    supported: 1
                }
            ),
            "{err}"
        );
        let c = Connection::open(&path).unwrap();
        let v: i64 = c
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 2);
        let ours: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'mirror_items'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ours, 0, "nothing was added to the newer file");
    }

    #[test]
    fn rows_round_trip_and_updates_replace_them() {
        let mut m = Manifest::open_in_memory().unwrap();
        let mut r = sample();
        m.put(&r).unwrap();
        r.state = ItemState::Verified;
        r.verify_level = VerifyLevel::L4RoundTrip;
        r.attempts = 0;
        r.next_attempt_at = None;
        r.upload_pending = false;
        m.put(&r).unwrap();
        assert_eq!(m.get("A!1").unwrap(), Some(r.clone()));
        assert_eq!(m.items().unwrap().len(), 1);
        assert!(m.get("nope").unwrap().is_none());
        let snap = m.snapshot().unwrap();
        assert_eq!(snap["A!1"], r);
    }

    #[test]
    fn a_failed_batch_writes_nothing() {
        let mut m = Manifest::open_in_memory().unwrap();
        m.conn
            .execute_batch(
                "CREATE TRIGGER no_bad BEFORE INSERT ON mirror_items \
                 WHEN NEW.source_id = 'bad' BEGIN SELECT RAISE(ABORT, 'no'); END;",
            )
            .unwrap();
        let ok = ItemRow::new("good", "g", ItemKind::File);
        let bad = ItemRow::new("bad", "b", ItemKind::File);
        assert!(m.put_many(&[ok, bad]).is_err());
        assert!(m.items().unwrap().is_empty());
    }

    #[test]
    fn runs_are_recorded() {
        let mut m = Manifest::open_in_memory().unwrap();
        let id = m.begin_run("dry", "2026-10-29T00:00:00Z").unwrap();
        let run = RunRow {
            id,
            mode: "dry".into(),
            started: "2026-10-29T00:00:00Z".into(),
            finished: Some("2026-10-29T00:00:01Z".into()),
            outcome: Some("completed".into()),
            files_mirrored: 2,
            bytes_uploaded: 10,
            ..Default::default()
        };
        m.finish_run(&run).unwrap();
        assert_eq!(m.runs(5).unwrap(), vec![run]);
        assert!(m
            .finish_run(&RunRow {
                id: 99,
                ..Default::default()
            })
            .is_err());
    }

    #[test]
    fn unknown_values_fail_closed() {
        let m = Manifest::open_in_memory().unwrap();
        m.conn
            .execute(
                "INSERT INTO mirror_items (source_id, source_path, kind, state) \
                 VALUES ('x', 'x', 'file', 'teleported')",
                [],
            )
            .unwrap();
        assert!(matches!(m.items(), Err(MirrorError::Corrupt(_))));
    }
}
