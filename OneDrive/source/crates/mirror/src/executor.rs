//! Carrying a plan out, one file at a time.
//!
//! For each file:
//!
//! 1. download it from the source into its own staging folder, checked
//!    against the source's digests, and record
//!    [`L1SourceVerified`](VerifyLevel::L1SourceVerified) with the SHA-1 and
//!    SHA-256 computed from the staged bytes;
//! 2. upload it as a *new* file, which never overwrites anything, and record
//!    [`L2Uploaded`](VerifyLevel::L2Uploaded);
//! 3. ask the destination what it holds and record
//!    [`L3RemoteClaimed`](VerifyLevel::L3RemoteClaimed) if its SHA-1 and size
//!    match ours;
//! 4. optionally download it back and record
//!    [`L4RoundTrip`](VerifyLevel::L4RoundTrip) if the SHA-256 matches.
//!
//! Each level is written to the manifest before the next step starts, and a
//! staged file is removed only after that. Before an upload the manifest is
//! marked `upload_pending`; if the run stops (or the process dies) before
//! the outcome is recorded, the next run asks the destination first and
//! adopts what it finds if identical, so nothing is uploaded twice.
//!
//! A file the destination already has with other content, or that changed
//! on the source after it was mirrored, is marked diverged and left alone
//! on both sides. Nothing is ever deleted, moved or renamed anywhere.
//!
//! With [`ExecOptions::parallel`] above 1, step 1 for the next few files
//! (looking the file up and downloading it into staging) runs ahead of the
//! rest, that many at a time; everything after it, and every manifest
//! write, still happens one file at a time and in plan order.
//!
//! The staging folder is used only if the mirror made it (recorded in the
//! manifest by device and inode) or it was empty when first used, and one
//! run at a time holds it, under an exclusive `flock` on the folder.

use std::collections::HashSet;
use std::ffi::CString;
use std::fs::File;
use std::future::Future;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axentra_clouddrive_provider::path::check_name;
use axentra_clouddrive_provider::{
    Capability, ContentDigests, ItemKind, MirrorDestination, ProviderError, RelPath, RemoteStat,
    SourceItem, SourceProvider, VerifyLevel,
};
use chrono::{DateTime, SecondsFormat, Utc};
use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tokio::sync::watch;

use crate::error::{MResult, MirrorError};
use crate::manifest::{ItemRow, ItemState, Manifest, RunRow};
use crate::planner::{content_changed, Action, ChangedSourcePolicy, Plan, PlanEntry};
use crate::seed::SeedIndex;

/// Staging folders made by the executor start with this; anything else in
/// the staging folder is left alone.
const STAGE_PREFIX: &str = "m-";

/// Manifest key recording the staging folder the mirror made or took over
/// (`<dev>:<inode> <path>`): only that folder is ever cleaned.
const STAGING_KEY: &str = "staging_dir";

pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// How many bytes are free for an unprivileged user on the filesystem
/// holding `path`.
pub type FreeSpace = fn(&Path) -> std::io::Result<u64>;

#[derive(Clone)]
pub struct ExecOptions {
    /// Private (0700) folder for files in transit, given as an absolute
    /// path. Created if missing; an existing one is used only if the mirror
    /// made it or it is empty.
    pub staging_dir: PathBuf,
    /// Leave at least this much free on the staging filesystem.
    pub min_free_bytes: u64,
    /// The first retry waits this long, doubling each time.
    pub backoff_base: Duration,
    pub backoff_max: Duration,
    pub clock: Clock,
    pub free_space: FreeSpace,
    /// How many files are looked up and downloaded at once, ahead of the
    /// upload. 1 does one file at a time.
    pub parallel: usize,
    /// When fetching ahead, take each file as the plan describes it
    /// instead of looking it up again: half the requests to a source that
    /// limits them. Still safe: the download is checked against the plan's
    /// digests, so a file changed since fails and waits for the next run
    /// (which plans it afresh), and one removed since fails as not found.
    pub trust_plan: bool,
    /// When the source asks to slow down, pause (at most 5 minutes at a
    /// time) and carry on with the next files instead of ending the run.
    /// The file that was turned away waits for the next run.
    pub wait_out_rate_limits: bool,
    /// Local copies to take files from, when fetching ahead, instead of
    /// downloading them: only a copy whose content matches the source's
    /// digests is taken.
    pub seed: Option<Arc<SeedIndex>>,
}

impl ExecOptions {
    pub fn new(staging_dir: PathBuf) -> Self {
        Self {
            staging_dir,
            min_free_bytes: 512 << 20,
            backoff_base: Duration::from_secs(60),
            backoff_max: Duration::from_secs(6 * 3600),
            clock: Arc::new(Utc::now),
            free_space: statvfs_free,
            parallel: 1,
            trust_plan: false,
            wait_out_rate_limits: false,
            seed: None,
        }
    }
}

/// Free space for an unprivileged user, from statvfs(3).
pub fn statvfs_free(path: &Path) -> std::io::Result<u64> {
    let c = CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: `c` is a valid NUL-terminated string and `st` a properly
    // sized, writable statvfs that the call fills in.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
}

/// Why a run ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RunOutcome {
    /// Every item in the plan was handled (some may have failed and wait
    /// for a retry).
    Completed,
    /// Stopped on request.
    Cancelled,
    /// A provider needs signing in again. Nothing was marked failed for it.
    AuthRequired,
    /// A provider asked to slow down; nothing more is tried before then.
    RateLimited {
        retry_at: i64,
    },
    Offline,
    /// The staging filesystem is too full to stage the next file.
    LowDiskSpace {
        free: u64,
        needed: u64,
    },
    /// Something not about one file (the destination folder, say).
    Failed {
        error: String,
    },
}

impl RunOutcome {
    pub fn as_str(&self) -> String {
        match self {
            RunOutcome::Completed => "completed".into(),
            RunOutcome::Cancelled => "cancelled".into(),
            RunOutcome::AuthRequired => "auth_required".into(),
            RunOutcome::RateLimited { retry_at } => format!("rate_limited until {retry_at}"),
            RunOutcome::Offline => "offline".into(),
            RunOutcome::LowDiskSpace { free, needed } => {
                format!("low_disk_space ({free} free, {needed} needed)")
            }
            RunOutcome::Failed { error } => format!("failed: {error}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: i64,
    pub outcome: RunOutcome,
    pub folders_created: u64,
    /// Uploaded by this run (new files and new revisions).
    pub files_mirrored: u64,
    /// Found at the destination with the same content and recorded.
    pub files_adopted: u64,
    /// Already uploaded before; only verified further this run.
    pub files_verified: u64,
    pub files_diverged: u64,
    pub files_skipped: u64,
    pub files_failed: u64,
    /// Waiting out a backoff from an earlier failure; not tried.
    pub files_waiting: u64,
    pub source_deleted: u64,
    pub bytes_uploaded: u64,
    /// The most the staging folder held at once, measured on disk.
    pub staging_peak: u64,
    /// Of `files_mirrored`, taken from a local copy instead of downloaded.
    #[serde(default)]
    pub files_seeded: u64,
}

impl RunSummary {
    fn new(run_id: i64) -> Self {
        Self {
            run_id,
            outcome: RunOutcome::Completed,
            folders_created: 0,
            files_mirrored: 0,
            files_adopted: 0,
            files_verified: 0,
            files_diverged: 0,
            files_skipped: 0,
            files_failed: 0,
            files_waiting: 0,
            source_deleted: 0,
            bytes_uploaded: 0,
            staging_peak: 0,
            files_seeded: 0,
        }
    }
}

/// What became of one file this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Done {
    Mirrored,
    Adopted,
    Verified,
    Diverged,
    Skipped,
    Failed,
    Deleted,
}

/// Carry on with the next item, or stop the run.
enum Step {
    Next(Done),
    Halt(RunOutcome),
}

macro_rules! step {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(s) => return Ok(s),
        }
    };
}

pub struct Executor<'a, S, D> {
    source: &'a S,
    dest: &'a D,
    manifest: &'a mut Manifest,
    opts: ExecOptions,
    /// Destination folders made sure of this run.
    ensured: HashSet<RelPath>,
    peak: u64,
    bytes_uploaded: u64,
    /// The staging folder, held under an exclusive flock for the run.
    staging_lock: Option<File>,
}

impl<'a, S: SourceProvider, D: MirrorDestination> Executor<'a, S, D> {
    pub fn new(source: &'a S, dest: &'a D, manifest: &'a mut Manifest, opts: ExecOptions) -> Self {
        Self {
            source,
            dest,
            manifest,
            opts,
            ensured: HashSet::new(),
            peak: 0,
            bytes_uploaded: 0,
            staging_lock: None,
        }
    }

    fn now(&self) -> DateTime<Utc> {
        (self.opts.clock)()
    }

    fn stamp(&self) -> String {
        self.now().to_rfc3339_opts(SecondsFormat::Secs, true)
    }

    /// Carry out `plan`. Once `stop` turns true it stops between steps; a
    /// download in progress (into staging only) is abandoned, but an
    /// upload in progress is let finish, so nothing is left half-made at
    /// the destination.
    pub async fn run(
        &mut self,
        plan: &Plan,
        mut stop: watch::Receiver<bool>,
    ) -> MResult<RunSummary> {
        if plan.options.round_trip {
            self.dest.capabilities().require(Capability::RoundTrip)?;
        }
        self.prepare_staging()?;
        let started = self.stamp();
        let run_id = self.manifest.begin_run("real", &started)?;
        let mut sum = RunSummary::new(run_id);
        let result = match self.apply_plan(plan, &mut sum) {
            Ok(()) => self.execute(plan, &mut stop, &mut sum).await,
            Err(e) => Err(e),
        };
        // Whatever happened, nothing is left staged.
        let cleaned = self.clean_staging();
        self.staging_lock = None;
        sum.staging_peak = self.peak;
        sum.bytes_uploaded = self.bytes_uploaded;
        let (outcome, err) = match result.and_then(|o| cleaned.map(|()| o)) {
            Ok(o) => (o, None),
            Err(e) => (
                RunOutcome::Failed {
                    error: e.to_string(),
                },
                Some(e),
            ),
        };
        sum.outcome = outcome;
        if sum.outcome == RunOutcome::Completed {
            if let Some(c) = &plan.source_cursor {
                self.manifest.kv_set("source_cursor", c)?;
            }
        }
        self.manifest.kv_set("dest_root", &plan.options.dest_root)?;
        let row = RunRow {
            id: run_id,
            mode: "real".into(),
            started,
            finished: Some(self.stamp()),
            outcome: Some(sum.outcome.as_str()),
            folders_created: sum.folders_created,
            files_mirrored: sum.files_mirrored,
            files_adopted: sum.files_adopted,
            files_diverged: sum.files_diverged,
            files_skipped: sum.files_skipped,
            files_failed: sum.files_failed,
            bytes_uploaded: sum.bytes_uploaded,
            staging_peak: sum.staging_peak,
        };
        self.manifest.finish_run(&row)?;
        match err {
            Some(e) => Err(e),
            None => Ok(sum),
        }
    }

    /// Record in one transaction what the plan found that needs no
    /// transfer: skips, divergences, deletions, moves, and new items.
    fn apply_plan(&mut self, plan: &Plan, sum: &mut RunSummary) -> MResult<()> {
        let mut rows = Vec::new();
        for e in &plan.entries {
            let mut row = match self.manifest.get(&e.source_id)? {
                Some(r) => r,
                None => ItemRow::new(&e.source_id, &e.source_path, e.kind()),
            };
            let reached = row.reached_destination();
            match e.action {
                Action::CreateFolder
                | Action::FolderExists
                | Action::UploadNew
                | Action::AdoptExisting
                | Action::CheckExisting => {
                    if !reached {
                        take_version(&mut row, e);
                        if matches!(
                            row.state,
                            ItemState::Skipped | ItemState::Diverged | ItemState::SourceDeleted
                        ) {
                            row.state = ItemState::Planned;
                            row.detail = None;
                        }
                    }
                }
                Action::Resume | Action::AlreadyMirrored | Action::UploadRevision => continue,
                Action::Diverged => {
                    if !reached {
                        take_version(&mut row, e);
                    }
                    row.state = ItemState::Diverged;
                    row.detail = e.reason.clone();
                    if !e.folder {
                        sum.files_diverged += 1;
                    }
                }
                Action::SkipTooLarge | Action::SkipUnsupported => {
                    if !reached {
                        take_version(&mut row, e);
                        row.state = ItemState::Skipped;
                    }
                    row.detail = e.reason.clone();
                    if !e.folder {
                        sum.files_skipped += 1;
                    }
                }
                Action::SourceDeleted => {
                    row.state = ItemState::SourceDeleted;
                    row.detail = e.reason.clone();
                    row.upload_pending = false;
                    sum.source_deleted += 1;
                }
                Action::SourceMoved => {
                    row.detail = e.reason.clone();
                }
            }
            rows.push(row);
        }
        self.manifest.put_many(&rows)
    }

    async fn execute(
        &mut self,
        plan: &Plan,
        stop: &mut watch::Receiver<bool>,
        sum: &mut RunSummary,
    ) -> MResult<RunOutcome> {
        let works = |e: &&PlanEntry| {
            if e.folder {
                matches!(
                    e.action,
                    Action::CreateFolder | Action::FolderExists | Action::Resume
                )
            } else {
                e.action.stages()
            }
        };
        let todo: Vec<&PlanEntry> = plan.entries.iter().filter(works).collect();
        // Each folder, and each file's parent folder before its upload, is
        // made sure of (with its parents) as it comes; nothing is created
        // for a run that has nothing to upload.
        // Folders, parents first (the plan is in that order).
        for e in todo.iter().filter(|e| e.folder) {
            if *stop.borrow() {
                return Ok(RunOutcome::Cancelled);
            }
            match self.do_folder(e, sum).await? {
                Step::Next(_) => {}
                Step::Halt(o) => return Ok(o),
            }
        }

        // Which files to fetch ahead: those with nothing at the destination
        // yet and not waiting out a backoff. The rest (and anything a fetch
        // ahead couldn't do) is done in turn by `do_file`.
        let now = self.now().timestamp();
        let mut files = Vec::new();
        for e in todo.iter().filter(|e| !e.folder) {
            let ahead = self.opts.parallel > 1
                && self.manifest.get(&e.source_id)?.is_some_and(|r| {
                    r.next_attempt_at.is_none_or(|t| t <= now)
                        && r.verify_level < VerifyLevel::L2Uploaded
                        && !r.adopted
                        && r.dest_uid.is_none()
                        && !r.upload_pending
                });
            files.push((*e, ahead));
        }
        let fetch = Fetch {
            source: self.source,
            staging_dir: self.opts.staging_dir.clone(),
            cap: plan.options.staging_cap,
            min_free: self.opts.min_free_bytes,
            free_space: self.opts.free_space,
            reserved: AtomicU64::new(0),
            trust_plan: self.opts.trust_plan,
            seed: self.opts.seed.clone(),
        };
        let fetch = &fetch;
        // The fetches run in their own future, beside the loop below, so
        // they keep going while a file is being written: polled only from
        // the loop, they would advance only while it waits for the next.
        let n = self.opts.parallel.max(1);
        let (tx, mut rx) = tokio::sync::mpsc::channel(n);
        let stop_ahead = stop.clone();
        let produce = async move {
            let mut ahead = stream::iter(files)
                .map(|(e, go)| {
                    let mut stop = stop_ahead.clone();
                    async move {
                        let pre = if go {
                            or_stop(&mut stop, fetch.prefetch(e)).await
                        } else {
                            None
                        };
                        (e, pre)
                    }
                })
                .buffered(n);
            while let Some(next) = ahead.next().await {
                if tx.send(next).await.is_err() {
                    break;
                }
            }
            // Everything is sent; `rx` ends once it has been taken.
            drop(tx);
            std::future::pending::<()>().await
        };
        let consume = async {
            while let Some((e, pre)) = rx.recv().await {
                if *stop.borrow() {
                    return Ok(RunOutcome::Cancelled);
                }
                let reserved = pre.as_ref().map_or(0, |p: &Prefetched| p.reserved);
                let seeded = pre.as_ref().is_some_and(|p| p.seeded);
                let Some(row) = self.manifest.get(&e.source_id)? else {
                    self.unstage(&e.source_id);
                    fetch.release(reserved);
                    continue;
                };
                if row
                    .next_attempt_at
                    .is_some_and(|t| t > self.now().timestamp())
                {
                    sum.files_waiting += 1;
                    self.unstage(&e.source_id);
                    fetch.release(reserved);
                    continue;
                }
                let step = self.do_file(plan, e, row, stop, pre).await;
                self.unstage(&e.source_id);
                fetch.release(reserved);
                match step? {
                    Step::Next(done) => match done {
                        Done::Mirrored => {
                            sum.files_mirrored += 1;
                            sum.files_seeded += u64::from(seeded);
                        }
                        Done::Adopted => sum.files_adopted += 1,
                        Done::Verified => sum.files_verified += 1,
                        Done::Diverged => sum.files_diverged += 1,
                        Done::Skipped => sum.files_skipped += 1,
                        Done::Failed => sum.files_failed += 1,
                        Done::Deleted => sum.source_deleted += 1,
                    },
                    Step::Halt(RunOutcome::RateLimited { retry_at })
                        if self.opts.wait_out_rate_limits =>
                    {
                        sum.files_waiting += 1;
                        let secs = (retry_at - self.now().timestamp()).clamp(1, 300) as u64;
                        let pause = tokio::time::sleep(Duration::from_secs(secs));
                        if or_stop(stop, pause).await.is_none() {
                            return Ok(RunOutcome::Cancelled);
                        }
                    }
                    Step::Halt(o) => return Ok(o),
                }
            }
            Ok(RunOutcome::Completed)
        };
        tokio::select! {
            r = consume => r,
            () = produce => unreachable!("produce never finishes"),
        }
    }

    async fn ensure_folder(&mut self, folder: &RelPath) -> Result<String, ProviderError> {
        let uid = self.dest.ensure_folder(folder).await?;
        // Its parents are there too now.
        let mut at = Some(folder.clone());
        while let Some(p) = at {
            at = p.parent();
            self.ensured.insert(p);
        }
        Ok(uid)
    }

    async fn do_folder(&mut self, e: &PlanEntry, sum: &mut RunSummary) -> MResult<Step> {
        let Some(mut row) = self.manifest.get(&e.source_id)? else {
            return Ok(Step::Next(Done::Skipped));
        };
        if row
            .next_attempt_at
            .is_some_and(|t| t > self.now().timestamp())
        {
            return Ok(Step::Next(Done::Skipped));
        }
        let target = parse_dest(&row, e)?;
        match self.ensure_folder(&target).await {
            Ok(uid) => {
                if e.action == Action::CreateFolder {
                    sum.folders_created += 1;
                }
                row.dest_uid = Some(uid);
                row.dest_path = Some(target.to_string());
                row.state = ItemState::Verified;
                row.first_mirrored_at.get_or_insert_with(|| self.stamp());
                row.last_verified_at = Some(self.stamp());
                clear_errors(&mut row);
                self.manifest.put(&row)?;
                Ok(Step::Next(Done::Verified))
            }
            Err(ProviderError::Conflict(why)) => {
                row.state = ItemState::Diverged;
                row.detail = Some(format!(
                    "something else is at the destination where this folder goes ({why}); both kept"
                ));
                self.manifest.put(&row)?;
                Ok(Step::Next(Done::Diverged))
            }
            Err(err) => self.fail(&mut row, err, "creating the folder"),
        }
    }

    async fn do_file(
        &mut self,
        plan: &Plan,
        e: &PlanEntry,
        mut row: ItemRow,
        stop: &mut watch::Receiver<bool>,
        pre: Option<Prefetched>,
    ) -> MResult<Step> {
        let target = parse_dest(&row, e)?;
        let cap = plan.options.staging_cap;
        let (looked_up, mut pre_staged) = match pre {
            Some(p) => (Some(p.item), p.staged),
            None => (None, None),
        };

        // What the source holds now, which may differ from the plan.
        let looked_up = match looked_up {
            Some(r) => r,
            None => self.source.item(&e.source_id).await,
        };
        let item = match looked_up {
            Ok(i) if !i.deleted => i,
            Ok(_) | Err(ProviderError::NotFound(_)) => {
                row.state = ItemState::SourceDeleted;
                row.upload_pending =
                    row.upload_pending && row.verify_level < VerifyLevel::L2Uploaded;
                row.detail = Some("gone from the source; the destination keeps any copy".into());
                self.manifest.put(&row)?;
                return Ok(Step::Next(Done::Deleted));
            }
            Err(err) => return self.fail(&mut row, err, "reading the source"),
        };
        if item.kind != ItemKind::File || item.special.is_some() {
            if !row.reached_destination() {
                row.state = ItemState::Skipped;
            }
            row.detail = Some("no longer a plain file on the source".into());
            self.manifest.put(&row)?;
            return Ok(Step::Next(Done::Skipped));
        }
        // A name the destination refuses never reached it, whatever an
        // earlier attempt recorded.
        if let Some(why) = target
            .name()
            .and_then(|n| self.dest.name_problem(n, ItemKind::File))
        {
            if row.verify_level < VerifyLevel::L2Uploaded && !row.adopted {
                row.upload_pending = false;
                row.state = ItemState::Skipped;
                clear_errors(&mut row);
            }
            row.detail = Some(format!("its name can't be used at the destination: {why}"));
            self.manifest.put(&row)?;
            return Ok(Step::Next(Done::Skipped));
        }

        // A mirrored file that has changed since.
        let mut revision = false;
        let has_copy = row.verify_level >= VerifyLevel::L2Uploaded || row.dest_uid.is_some();
        if has_copy && content_changed(&row, &item.digests, item.version.as_deref()) {
            match plan.options.on_source_change {
                ChangedSourcePolicy::MarkDiverged => {
                    row.state = ItemState::Diverged;
                    row.detail = Some(
                        "changed on the source after it was mirrored; destination left as it was"
                            .into(),
                    );
                    self.manifest.put(&row)?;
                    return Ok(Step::Next(Done::Diverged));
                }
                ChangedSourcePolicy::CreateNewRevision => revision = true,
            }
        }
        // A revision whose upload was interrupted (L1 of the new version
        // recorded, the old copy still there).
        let revision_resume = row.dest_uid.is_some() && row.verify_level < VerifyLevel::L2Uploaded;

        let mut done = Done::Verified;
        if revision || (row.verify_level < VerifyLevel::L2Uploaded && !row.adopted) {
            if revision || !has_copy {
                take_item_version(&mut row, &item);
            }
            if item.digests.size > cap {
                row.state = if has_copy {
                    ItemState::Diverged
                } else {
                    ItemState::Skipped
                };
                row.detail = Some(format!(
                    "{} bytes is more than the staging cap of {cap} bytes",
                    item.digests.size
                ));
                self.manifest.put(&row)?;
                return Ok(Step::Next(Done::Skipped));
            }
            let staged = step!(
                self.stage_source(&item, &mut row, stop, pre_staged.take())
                    .await?
            );

            let parent = target.parent().unwrap_or_default();
            if !self.ensured.contains(&parent) {
                if let Err(err) = self.ensure_folder(&parent).await {
                    return self.fail(&mut row, err, "creating the parent folder");
                }
            }

            if revision || revision_resume {
                if row.upload_pending {
                    match self.dest.stat(&target).await {
                        Ok(st) if claims_match(&st, &row) => {
                            self.record_claim(&mut row, &st);
                            row.upload_pending = false;
                            self.manifest.put(&row)?;
                            done = Done::Mirrored;
                        }
                        Ok(_) | Err(ProviderError::NotFound(_)) => {}
                        Err(err) => return self.fail(&mut row, err, "checking the destination"),
                    }
                }
                if row.verify_level < VerifyLevel::L2Uploaded {
                    row.upload_pending = true;
                    self.manifest.put(&row)?;
                    // Not abandoned on a stop request: killing it midway
                    // could leave a half-made revision behind.
                    match self.dest.upload_revision(&target, &staged).await {
                        Ok(st) => {
                            self.record_upload(&mut row, &st)?;
                            done = Done::Mirrored;
                        }
                        Err(err) => return self.fail(&mut row, err, "uploading a revision"),
                    }
                }
            } else {
                // Whether an earlier run may have uploaded it already.
                let was_pending = row.upload_pending;
                let look_first = row.upload_pending
                    || matches!(e.action, Action::AdoptExisting | Action::CheckExisting);
                let mut exists = None;
                if look_first {
                    match self.dest.stat(&target).await {
                        Ok(st) => exists = Some(st),
                        Err(ProviderError::NotFound(_)) => {}
                        Err(err) => return self.fail(&mut row, err, "checking the destination"),
                    }
                }
                if exists.is_none() {
                    row.upload_pending = true;
                    self.manifest.put(&row)?;
                    let name = target.name().unwrap_or_default().to_string();
                    // Not abandoned on a stop request: killing it midway
                    // could leave a half-made file behind.
                    match self.dest.upload_new(&parent, &name, &staged).await {
                        Ok(st) => {
                            self.record_upload(&mut row, &st)?;
                            done = Done::Mirrored;
                        }
                        // Taken since the plan (or hidden from it): this
                        // attempt put nothing there.
                        Err(ProviderError::Conflict(_)) => match self.dest.stat(&target).await {
                            Ok(st) => exists = Some(st),
                            Err(err) => {
                                return self.fail(&mut row, err, "checking the destination")
                            }
                        },
                        Err(err) => return self.fail(&mut row, err, "uploading"),
                    }
                }
                if let Some(st) = exists {
                    // Ours only if an earlier run's upload was interrupted.
                    let ours = was_pending;
                    row.upload_pending = false;
                    if claims_match(&st, &row) {
                        self.record_claim(&mut row, &st);
                        row.adopted = !ours;
                        row.first_mirrored_at.get_or_insert_with(|| self.stamp());
                        row.detail = Some(if ours {
                            "found at the destination after an interrupted upload".into()
                        } else {
                            "already at the destination with the same content; adopted".into()
                        });
                        self.manifest.put(&row)?;
                        done = if ours { Done::Mirrored } else { Done::Adopted };
                    } else {
                        row.state = ItemState::Diverged;
                        row.detail = Some(if st.claimed.sha1.is_none() {
                            "a file of the same name is at the destination and it reports no \
                             SHA-1 to compare; both kept"
                                .into()
                        } else {
                            "a different file of the same name is at the destination; both kept"
                                .into()
                        });
                        self.manifest.put(&row)?;
                        return Ok(Step::Next(Done::Diverged));
                    }
                }
            }
        }

        // L3: what the destination claims to hold.
        if row.verify_level == VerifyLevel::L2Uploaded {
            match self.dest.stat(&target).await {
                Ok(st) => match &st.claimed.sha1 {
                    Some(_) if claims_match(&st, &row) => {
                        self.record_claim(&mut row, &st);
                        self.manifest.put(&row)?;
                    }
                    Some(theirs) => {
                        let msg = format!(
                            "the destination claims SHA-1 {theirs} and {} bytes, but {} and {} \
                             bytes were uploaded",
                            st.claimed.size,
                            row.sha1.as_deref().unwrap_or("?"),
                            row.size
                        );
                        row.dest_claimed_sha1 = Some(theirs.clone());
                        return self.fail_verify(&mut row, msg);
                    }
                    None => {
                        row.detail = Some(
                            "the destination reports no SHA-1; remote-claimed not reached".into(),
                        );
                        self.manifest.put(&row)?;
                    }
                },
                Err(err) => return self.fail(&mut row, err, "checking the upload"),
            }
        }
        // The source copy is no longer needed: its level is recorded.
        self.unstage(&row.source_id);

        // L4: download it back and compare.
        if plan.options.round_trip
            && row.verify_level >= VerifyLevel::L2Uploaded
            && row.verify_level < VerifyLevel::L4RoundTrip
        {
            let Some(want) = row.sha256.clone() else {
                return self.fail_verify(&mut row, "no SHA-256 was recorded to compare".into());
            };
            if let Err(s) = self.check_space(row.size) {
                return Ok(s);
            }
            let dir = self.stage_dir(&row.source_id)?;
            let back = dir.join(target.name().unwrap_or("file"));
            match or_stop(stop, self.dest.download_to(&target, &back)).await {
                None => return Ok(Step::Halt(RunOutcome::Cancelled)),
                Some(Ok(got)) => {
                    self.measure();
                    if got.size == row.size
                        && got
                            .sha256
                            .as_deref()
                            .is_some_and(|g| g.eq_ignore_ascii_case(&want))
                    {
                        row.verify_level = VerifyLevel::L4RoundTrip;
                        row.last_verified_at = Some(self.stamp());
                        self.manifest.put(&row)?;
                    } else {
                        let msg = format!(
                            "round trip: what came back has SHA-256 {} and {} bytes, expected {want} \
                             and {} bytes",
                            got.sha256.as_deref().unwrap_or("?"),
                            got.size,
                            row.size
                        );
                        return self.fail_verify(&mut row, msg);
                    }
                }
                Some(Err(ProviderError::Integrity(why))) => {
                    return self.fail_verify(&mut row, format!("round trip: {why}"));
                }
                Some(Err(err)) => return self.fail(&mut row, err, "downloading it back"),
            }
            self.unstage(&row.source_id);
        }

        let target_level = plan.options.target_level();
        if row.verify_level >= target_level
            || (!plan.options.round_trip && row.verify_level == VerifyLevel::L3RemoteClaimed)
        {
            row.state = ItemState::Verified;
        } else if row.verify_level >= VerifyLevel::L2Uploaded {
            row.state = ItemState::Uploaded;
        }
        clear_errors(&mut row);
        self.manifest.put(&row)?;
        Ok(Step::Next(done))
    }

    /// Download the source's current version into staging and record L1.
    /// Returns where it is.
    async fn stage_source(
        &mut self,
        item: &SourceItem,
        row: &mut ItemRow,
        stop: &mut watch::Receiver<bool>,
        fetched: Option<(PathBuf, Result<ContentDigests, ProviderError>)>,
    ) -> MResult<Result<PathBuf, Step>> {
        if let Err(e) = check_name(&item.name) {
            return Ok(Err(self.fail(row, e.into(), "staging")?));
        }
        // Fetched ahead (for this same version of the item), or now.
        let (path, got) = match fetched {
            Some((path, got)) if path.file_name() == Some(item.name.as_ref()) => (path, got),
            _ => {
                if let Err(s) = self.check_space(item.digests.size) {
                    return Ok(Err(s));
                }
                let dir = self.stage_dir(&item.id)?;
                // Named exactly as on the source: some destinations name
                // the upload after the local file.
                let path = dir.join(&item.name);
                match or_stop(stop, self.source.download_to(item, &path)).await {
                    None => return Ok(Err(Step::Halt(RunOutcome::Cancelled))),
                    Some(got) => {
                        if got.is_ok() {
                            keep_modified(&path, item.modified);
                        }
                        (path, got)
                    }
                }
            }
        };
        let got = match got {
            Ok(d) => d,
            Err(err) => return Ok(Err(self.fail(row, err, "downloading from the source")?)),
        };
        self.measure();
        // Checked here too, whatever the source did: computed against claimed.
        if let Err(why) = got.verify_against(&item.digests) {
            let err = ProviderError::Integrity(format!("staged copy: {why}"));
            return Ok(Err(self.fail(row, err, "checking the download")?));
        }
        row.size = got.size;
        row.sha1 = got.sha1;
        row.sha256 = got.sha256;
        row.verify_level = VerifyLevel::L1SourceVerified;
        row.state = ItemState::Staged;
        self.manifest.put(row)?;
        Ok(Ok(path))
    }

    fn record_upload(&mut self, row: &mut ItemRow, st: &RemoteStat) -> MResult<()> {
        row.dest_uid = Some(st.uid.clone());
        row.verify_level = VerifyLevel::L2Uploaded;
        row.state = ItemState::Uploaded;
        row.upload_pending = false;
        row.adopted = false;
        row.first_mirrored_at.get_or_insert_with(|| self.stamp());
        self.bytes_uploaded += row.size;
        self.manifest.put(row)
    }

    /// The destination's claim matched: L3.
    fn record_claim(&self, row: &mut ItemRow, st: &RemoteStat) {
        row.dest_uid = Some(st.uid.clone());
        row.dest_claimed_sha1 = st.claimed.sha1.clone();
        row.verify_level = VerifyLevel::L3RemoteClaimed;
        row.state = ItemState::Uploaded;
        row.last_verified_at = Some(self.stamp());
    }

    /// Record a failed step with a backoff, or stop the run for errors that
    /// are about the provider rather than this item.
    fn fail(&mut self, row: &mut ItemRow, err: ProviderError, what: &str) -> MResult<Step> {
        let now = self.now();
        row.last_error = Some(format!("{what}: {err}"));
        let halt = halt_for(&err, now);
        match &err {
            ProviderError::AuthRequired | ProviderError::Offline(_) => {}
            ProviderError::RateLimited { retry_after } => {
                row.attempts = row.attempts.saturating_add(1);
                let wait = self
                    .backoff(row.attempts)
                    .max(retry_after.unwrap_or_default());
                row.next_attempt_at = Some(now.timestamp() + wait.as_secs() as i64);
            }
            _ => {
                row.attempts = row.attempts.saturating_add(1);
                row.next_attempt_at =
                    Some(now.timestamp() + self.backoff(row.attempts).as_secs() as i64);
                row.state = ItemState::Failed;
            }
        }
        self.manifest.put(row)?;
        Ok(match halt {
            Some(RunOutcome::RateLimited { .. }) => Step::Halt(RunOutcome::RateLimited {
                retry_at: row.next_attempt_at.unwrap_or(now.timestamp()),
            }),
            Some(o) => Step::Halt(o),
            None => Step::Next(Done::Failed),
        })
    }

    /// A verification that found a mismatch. The level stays where it was;
    /// nothing is uploaded again.
    fn fail_verify(&mut self, row: &mut ItemRow, msg: String) -> MResult<Step> {
        let now = self.now();
        row.attempts = row.attempts.saturating_add(1);
        row.next_attempt_at = Some(now.timestamp() + self.backoff(row.attempts).as_secs() as i64);
        row.last_error = Some(msg);
        row.state = ItemState::Failed;
        self.manifest.put(row)?;
        Ok(Step::Next(Done::Failed))
    }

    fn backoff(&self, attempts: u32) -> Duration {
        let factor = 1u32
            .checked_shl(attempts.saturating_sub(1))
            .unwrap_or(u32::MAX);
        self.opts
            .backoff_base
            .saturating_mul(factor)
            .min(self.opts.backoff_max)
    }

    // ----- staging ---------------------------------------------------------

    /// Make (or check) the staging folder, take this run's lock on it, and
    /// clear what an earlier run left. A folder the mirror didn't make is
    /// used only if empty, so nothing of anyone else's is ever removed or
    /// has its permissions changed.
    fn prepare_staging(&mut self) -> MResult<()> {
        let dir = self.opts.staging_dir.clone();
        if !dir.is_absolute() {
            return Err(MirrorError::Staging(format!(
                "{} is not an absolute path",
                dir.display()
            )));
        }
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let created = match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(e) => return Err(e.into()),
        };
        let meta = std::fs::symlink_metadata(&dir)?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(MirrorError::Staging(format!(
                "{} is not a plain folder",
                dir.display()
            )));
        }
        let id = format!("{}:{} {}", meta.dev(), meta.ino(), dir.display());
        if !created
            && self.manifest.kv_get(STAGING_KEY)?.as_deref() != Some(id.as_str())
            && std::fs::read_dir(&dir)?.next().is_some()
        {
            return Err(MirrorError::Staging(format!(
                "{} already holds files and wasn't made by the mirror; set [mirror] \
                 staging_dir to a new or empty folder",
                dir.display()
            )));
        }
        let lock = File::open(&dir)?;
        // SAFETY: a valid open descriptor, kept open in `staging_lock`.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::WouldBlock {
                return Err(MirrorError::Staging(format!(
                    "another mirror run is using {}; wait for it to finish",
                    dir.display()
                )));
            }
            return Err(e.into());
        }
        self.staging_lock = Some(lock);
        self.manifest.kv_set(STAGING_KEY, &id)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        self.clean_staging()
    }

    /// Remove every staging folder this executor makes, left by this run or
    /// an earlier one that stopped.
    fn clean_staging(&self) -> MResult<()> {
        for ent in std::fs::read_dir(&self.opts.staging_dir)? {
            let ent = ent?;
            let name = ent.file_name();
            if !name.as_bytes().starts_with(STAGE_PREFIX.as_bytes()) {
                continue;
            }
            let meta = std::fs::symlink_metadata(ent.path())?;
            if meta.is_dir() {
                std::fs::remove_dir_all(ent.path())?;
            } else {
                std::fs::remove_file(ent.path())?;
            }
        }
        Ok(())
    }

    /// A fresh, empty, private folder for one item.
    fn stage_dir(&self, source_id: &str) -> MResult<PathBuf> {
        let dir = self.opts.staging_dir.join(stage_name(source_id));
        self.unstage(source_id);
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        Ok(dir)
    }

    fn unstage(&self, source_id: &str) {
        let dir = self.opts.staging_dir.join(stage_name(source_id));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Whether `size` more bytes may be staged now.
    fn check_space(&self, size: u64) -> Result<(), Step> {
        let free = match (self.opts.free_space)(&self.opts.staging_dir) {
            Ok(f) => f,
            Err(e) => {
                return Err(Step::Halt(RunOutcome::Failed {
                    error: format!("checking free space: {e}"),
                }))
            }
        };
        let needed = size.saturating_add(self.opts.min_free_bytes);
        if free < needed {
            return Err(Step::Halt(RunOutcome::LowDiskSpace { free, needed }));
        }
        Ok(())
    }

    /// Note how much the staging folder holds now.
    fn measure(&mut self) {
        let used = dir_usage(&self.opts.staging_dir);
        self.peak = self.peak.max(used);
    }
}

/// Bytes in regular files under `dir`, not following links.
fn dir_usage(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for ent in rd.flatten() {
            let Ok(meta) = std::fs::symlink_metadata(ent.path()) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(ent.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    total
}

/// A file looked up, and downloaded into staging, ahead of its turn.
struct Prefetched {
    item: Result<SourceItem, ProviderError>,
    /// Where it was staged and what the download gave; `None` if it wasn't
    /// (not a plain file, too big, short of space): its turn does that.
    staged: Option<(PathBuf, Result<ContentDigests, ProviderError>)>,
    /// Bytes counted against free space until its turn is over.
    reserved: u64,
    /// Staged from a local copy, not downloaded.
    seeded: bool,
}

/// What fetching ahead needs, borrowed from nothing that the run changes.
struct Fetch<'a, S> {
    source: &'a S,
    staging_dir: PathBuf,
    cap: u64,
    min_free: u64,
    free_space: FreeSpace,
    /// Bytes fetched ahead and not yet done with.
    reserved: AtomicU64,
    /// [`ExecOptions::trust_plan`].
    trust_plan: bool,
    seed: Option<Arc<SeedIndex>>,
}

impl<S: SourceProvider> Fetch<'_, S> {
    async fn prefetch(&self, e: &PlanEntry) -> Prefetched {
        let id = e.source_id.as_str();
        let item = if self.trust_plan {
            Ok(planned_item(e))
        } else {
            self.source.item(id).await
        };
        let mut out = Prefetched {
            item,
            staged: None,
            reserved: 0,
            seeded: false,
        };
        let Ok(it) = &out.item else {
            return out;
        };
        let size = it.digests.size;
        if it.deleted
            || it.kind != ItemKind::File
            || it.special.is_some()
            || size > self.cap
            || check_name(&it.name).is_err()
        {
            return out;
        }
        let held = self.reserved.fetch_add(size, Ordering::SeqCst);
        out.reserved = size;
        let room = (self.free_space)(&self.staging_dir).is_ok_and(|free| {
            free >= size
                .saturating_add(held)
                .saturating_add(self.min_free)
        });
        if !room {
            return out;
        }
        let dir = self.staging_dir.join(stage_name(id));
        let _ = std::fs::remove_dir_all(&dir);
        if std::fs::DirBuilder::new().mode(0o700).create(&dir).is_err() {
            return out;
        }
        let path = dir.join(&it.name);
        if let Some(seed) = &self.seed {
            if let Some(got) = seed.take(&it.name, &it.digests, &path).await {
                keep_modified(&path, it.modified);
                out.staged = Some((path, Ok(got)));
                out.seeded = true;
                return out;
            }
        }
        let got = self.source.download_to(it, &path).await;
        if got.is_ok() {
            keep_modified(&path, it.modified);
        }
        out.staged = Some((path, got));
        out
    }

    fn release(&self, bytes: u64) {
        self.reserved.fetch_sub(bytes, Ordering::SeqCst);
    }
}

/// The file `e` describes, as the plan found it.
fn planned_item(e: &PlanEntry) -> SourceItem {
    SourceItem {
        id: e.source_id.clone(),
        name: e
            .source_path
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string(),
        parent_id: None,
        kind: e.kind(),
        deleted: false,
        special: None,
        digests: e.digests.clone(),
        modified: e.modified,
        version: e.version.clone(),
    }
}

/// Give a staged file the source's modification time, so a destination
/// that keeps file times (a local drive) shows the original one.
fn keep_modified(path: &Path, modified: Option<DateTime<Utc>>) {
    if let Some(m) = modified {
        if let Ok(f) = File::options().write(true).open(path) {
            let _ = f.set_modified(m.into());
        }
    }
}

fn stage_name(source_id: &str) -> String {
    let h = Sha1::digest(source_id.as_bytes());
    let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
    format!("{STAGE_PREFIX}{hex}")
}

/// Errors that end the run rather than count against one item.
fn halt_for(err: &ProviderError, now: DateTime<Utc>) -> Option<RunOutcome> {
    match err {
        ProviderError::AuthRequired => Some(RunOutcome::AuthRequired),
        ProviderError::Offline(_) => Some(RunOutcome::Offline),
        ProviderError::RateLimited { retry_after } => Some(RunOutcome::RateLimited {
            retry_at: now.timestamp() + retry_after.unwrap_or_default().as_secs() as i64,
        }),
        _ => None,
    }
}

/// Run `fut` unless `stop` turns true first; `None` if it did. A dropped
/// sender never stops anything.
async fn or_stop<T>(stop: &mut watch::Receiver<bool>, fut: impl Future<Output = T>) -> Option<T> {
    if *stop.borrow() {
        return None;
    }
    let stopped = async {
        loop {
            if *stop.borrow_and_update() {
                return;
            }
            if stop.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    };
    tokio::select! {
        biased;
        v = fut => Some(v),
        () = stopped => None,
    }
}

/// The destination's claimed SHA-1 and size match what was computed from
/// the staged bytes. No claimed SHA-1 is no match.
fn claims_match(st: &RemoteStat, row: &ItemRow) -> bool {
    match (&st.claimed.sha1, &row.sha1) {
        (Some(theirs), Some(ours)) => {
            theirs.eq_ignore_ascii_case(ours) && st.claimed.size == row.size
        }
        _ => false,
    }
}

fn clear_errors(row: &mut ItemRow) {
    row.attempts = 0;
    row.next_attempt_at = None;
    row.last_error = None;
}

fn parse_dest(row: &ItemRow, e: &PlanEntry) -> MResult<RelPath> {
    let s = row.dest_path.as_deref().unwrap_or(&e.dest_path);
    RelPath::parse(s).map_err(|err| MirrorError::Invalid(format!("destination path: {err}")))
}

/// Make the row describe the version in the plan, forgetting digests
/// computed for any earlier one.
fn take_version(row: &mut ItemRow, e: &PlanEntry) {
    row.source_path = e.source_path.clone();
    row.dest_path = Some(e.dest_path.clone());
    set_version(row, &e.digests, e.version.clone(), e.modified);
}

fn take_item_version(row: &mut ItemRow, item: &SourceItem) {
    set_version(row, &item.digests, item.version.clone(), item.modified);
}

fn set_version(
    row: &mut ItemRow,
    d: &ContentDigests,
    version: Option<String>,
    modified: Option<DateTime<Utc>>,
) {
    if content_changed(row, d, version.as_deref()) || row.qxh.is_none() && row.ctag.is_none() {
        row.sha1 = None;
        row.sha256 = None;
        if row.verify_level > VerifyLevel::L0Planned && row.verify_level < VerifyLevel::L2Uploaded {
            row.verify_level = VerifyLevel::L0Planned;
        }
    }
    row.size = d.size;
    row.qxh = d.qxh.clone();
    row.ctag = version;
    row.mtime = modified.map(|m| m.to_rfc3339_opts(SecondsFormat::Secs, true));
}
