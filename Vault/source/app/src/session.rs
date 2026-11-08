//! The open vault: views and group tree, ranked search, entry list, details with previous
//! passwords, archive (tombstone), editing, safe saving, idle auto-lock and passkey setup.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use eframe::egui::{self, Color32, Key, RichText};
use egui_extras::{Column, TableBuilder};
use vault_core::backups::BackupConfig;
use vault_core::categorize::{self, Area, Category, Source};
use vault_core::envelope::{self, KeyRing, RingKey};
use vault_core::form::{CustomField, EntryForm};
use vault_core::generator::{self, GeneratorOptions};
use vault_core::keepass::db::{Entry, EntryId, GroupId, GroupRef, Times, fields};
use vault_core::keyfile;
use vault_core::organize::{self, Issue};
use vault_core::rekey::{self, StageError, Staged};
use vault_core::search::{self, SearchOptions};
use vault_core::vault::{Credentials, KeyChangePreview, SealedVault, Vault, VaultError};
use zeroize::Zeroizing;

use crate::columns::{self, Col};
use crate::icon::{self, Icon};
use crate::jobs::{self, ExistingKey, Job, NewPasskey, UnlockError, WindowHandle};

mod backups;
mod floating;

const IDLE_LOCK: Duration = Duration::from_secs(5 * 60);
const RECENT_DAYS: i64 = 30;
/// Shown while Windows walks through registering a key (two prompts).
const PASSKEY_BUSY: &str =
    "Follow the Windows prompts: register, then confirm (phone or security key)…";
/// Shown while a key change re-encrypts the backups.
const KEY_CHANGE_BUSY: &str = "Re-encrypting backups for the new key…";
/// How long Cancel, a lock or closing waits for a key change's staging to stop.
const KEY_CHANGE_STOP_WAIT: Duration = Duration::from_secs(5);
const KEY_CHANGE_CANCELLED: &str = "Key change cancelled; nothing changed.";
/// Why Save is off in a backup opened as a vault.
const READ_ONLY_SAVE: &str = "A backup opened read-only: changes here aren't saved";
/// The database's credential record named other credentials than the ones that opened it.
const OUTSIDE_CHANGE: &str = "This vault's password or key was changed outside Axentra Vault (in KeePass, KeePassXC or the plugin). Backups made before that need the previous credentials (Settings > Backups).";
/// Key changes need the vault itself.
const READ_ONLY_KEYS: &str =
    "This is a backup opened read-only. Open your vault to change its unlock keys.";

/// Floating layout navigation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Vault,
    Favorites,
    Tags,
    Generator,
    Watchtower,
    Settings,
}

/// Something done this session, for the Recent Activity panel. Kept in memory only and holds
/// no secret values, just what happened to which entry title.
struct Activity {
    at: Instant,
    icon: Icon,
    text: String,
}

pub enum SessionAction {
    None,
    Copy(Zeroizing<String>),
    Lock,
    Close,
    /// Switch between the Floating and Classic layouts.
    SetLayout(crate::settings::Layout),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    All,
    Recent,
    Attention,
    Archived,
    Area(Area),
    Category(Category),
    Group(GroupId),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Title,
    Modified,
}

enum Confirm {
    Overwrite,
    /// "Save anyway" found a file it can't read as a KeePass database; why.
    ReplaceUnreadable(String),
    LockUnsaved,
    CloseUnsaved,
    Delete(EntryId),
}

struct Editor {
    id: Option<EntryId>,
    group: GroupId,
    form: EntryForm,
    reveal: bool,
    reveal_notes: bool,
    /// Indexes of protected custom fields currently shown in clear.
    revealed_fields: std::collections::HashSet<usize>,
    generator: GeneratorOptions,
}

struct GroupNode {
    id: GroupId,
    name: String,
    active: usize,
    children: Vec<GroupNode>,
}

struct Row {
    id: EntryId,
    title: String,
    username: String,
    host: String,
    modified: Option<i64>,
    modified_text: String,
    category: &'static str,
    /// Why it is listed: search match fields, health issues, or archive date.
    note: String,
    dim: bool,
    /// Text for the optional columns that are shown (group, tags, dates, notes, custom fields).
    extra: HashMap<Col, String>,
}

pub struct Session {
    pub vault: Vault,
    pub error: Option<String>,
    pub allow_close: bool,
    pub recovery: Option<SealedVault>,
    pub lock_notice: Option<String>,
    /// Windows antivirus / Smart App Control state, read when the vault opens.
    pub security: Option<crate::hardening::SecurityStatus>,
    view: View,
    sort: Sort,
    /// Classic entry-table columns (after Title).
    columns: Vec<Col>,
    /// Classic table sort: the column (None = Title) and whether it is descending.
    table_sort: (Option<Col>, bool),
    selected: Option<EntryId>,
    search: String,
    search_opts: SearchOptions,
    reveal: bool,
    reveal_previous: bool,
    reveal_notes: bool,
    reveal_fields: bool,
    editor: Option<Editor>,
    confirm: Option<Confirm>,
    passkey_job: Option<Job<Result<NewPasskey, UnlockError>>>,
    rotation_job: Option<Job<Result<jobs::RotatedKeys, UnlockError>>>,
    /// A key change re-encrypting the backups before it commits (shown as a modal).
    key_change: Option<KeyChange>,
    /// Running breach check and its progress.
    breach_job: Option<(
        Job<crate::breach::BreachResult>,
        std::sync::Arc<crate::siteicons::Progress>,
    )>,

    /// SHA-1 of each breached password, mapped to times seen in breaches. In memory for this session
    /// only; `None` until a check has run.
    breached: Option<HashMap<String, u64>>,
    /// Labels of the passkeys / security keys in the `.axvault` file (empty if none).
    unlock_keys: Vec<String>,
    last_activity: Instant,
    notice: Option<(String, Instant)>,
    /// Smart category per entry, recomputed after edits and every couple of seconds.
    categories: HashMap<EntryId, (Category, Source)>,
    categories_at: Option<Instant>,
    // Floating layout state.
    page: Page,
    tag_filter: Option<String>,
    tag_input: Option<String>,
    show_security: bool,
    activity: Vec<Activity>,
    generator: GeneratorOptions,
    generated: Zeroizing<String>,
    /// Site icon download in progress: the job, its progress, and the one entry to replace an
    /// icon for (None = fill in entries that have no icon yet).
    icon_job: Option<IconJob>,
    /// Custom icon PNG bytes by icon id, shared with the image loader.
    icon_cache: HashMap<uuid::Uuid, std::sync::Arc<[u8]>>,
    /// Settings > Backups (see `session/backups.rs`).
    backups: backups::Backups,
}

struct IconJob {
    job: Job<crate::siteicons::IconResults>,
    progress: std::sync::Arc<crate::siteicons::Progress>,
    only: Option<EntryId>,
}

/// What follows once a key change's staging has stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AfterCancel {
    Stay,
    Lock,
    Close,
}

/// A key change whose backups are being re-encrypted on a worker (see `vault_core::rekey`).
/// Nothing on disk has changed except the new backup key file until it commits; stopping it
/// (Cancel, a lock, closing the window) throws the staged copies away, and the new backup key
/// file with them (it opens nothing).
struct KeyChange {
    job: Job<Result<Staged, StageError>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<rekey::Progress>,
    started: Instant,
    /// The ring that goes beside the database when the change commits.
    ring: KeyRing,
    /// The new backup key file, already written.
    backup: PathBuf,
    /// Its contents' SHA-256 as written: only that file is deleted if the change stops.
    backup_sha256: Option<String>,
    /// Once asked to stop: since when, and what to do after.
    stopping: Option<(Instant, AfterCancel)>,
    /// The window's close button was pressed; the dialog asks whether to cancel and close.
    close_asked: bool,
}

impl KeyChange {
    fn new(
        job: Job<Result<Staged, StageError>>,
        cancel: Arc<AtomicBool>,
        progress: Arc<rekey::Progress>,
        ring: KeyRing,
        backup: PathBuf,
    ) -> KeyChange {
        KeyChange {
            job,
            cancel,
            progress,
            started: Instant::now(),
            ring,
            backup_sha256: vault_core::backups::sha256_file(&backup).ok(),
            backup,
            stopping: None,
            close_asked: false,
        }
    }

    /// Deletes the new backup key file of a key change that didn't commit (it opens nothing),
    /// if it still holds what was written. Returns what to tell the user if it couldn't be.
    fn discard_backup(&self) -> Option<String> {
        let ours = self.backup_sha256.as_ref().is_some_and(|written| {
            vault_core::backups::sha256_file(&self.backup).is_ok_and(|now| now == *written)
        });
        if !ours || matches!(vault_core::fsops::remove_file(&self.backup), Ok(true)) {
            return None;
        }
        self.backup.exists().then(|| {
            format!(
                " Delete {}: it does not open this vault.",
                self.backup.display()
            )
        })
    }

    /// "Key change cancelled; nothing changed.", once the new backup key file is gone.
    fn cancelled(&self) -> String {
        match self.discard_backup() {
            None => KEY_CHANGE_CANCELLED.into(),
            Some(note) => format!("Key change cancelled; the vault is unchanged.{note}"),
        }
    }

    /// How long staging may hold off the idle lock: 60 s plus 30 s per backup, at most 30 min.
    fn ceiling(&self) -> Duration {
        let n = self.progress.total() as u64;
        Duration::from_secs(60 + 30 * n).min(Duration::from_secs(30 * 60))
    }

    /// Asks the worker to stop before its next key derivation. A lock or close outranks a
    /// plain Cancel already under way.
    fn stop(&mut self, after: AfterCancel) {
        self.cancel.store(true, Ordering::SeqCst);
        match &mut self.stopping {
            Some((_, then)) if after != AfterCancel::Stay => *then = after,
            Some(_) => {}
            None => self.stopping = Some((Instant::now(), after)),
        }
    }
}

impl Drop for KeyChange {
    /// A session that goes away stops its worker; an unpublished result deletes its copies.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

fn build_tree(g: GroupRef<'_>) -> GroupNode {
    GroupNode {
        id: g.id(),
        name: g.name.clone(),
        active: g.entries().filter(|e| !organize::is_archived(e)).count(),
        children: g.groups().map(build_tree).collect(),
    }
}

/// "Banking / Cards" style paths for the group picker, recycle bin excluded.
fn group_paths(
    node: &GroupNode,
    prefix: &str,
    bin: Option<GroupId>,
    out: &mut Vec<(GroupId, String)>,
) {
    if Some(node.id) == bin {
        return;
    }
    let path = if prefix.is_empty() {
        node.name.clone()
    } else {
        format!("{prefix} / {}", node.name)
    };
    out.push((node.id, path.clone()));
    for c in &node.children {
        group_paths(c, &path, bin, out);
    }
}

fn fmt_date(t: Option<NaiveDateTime>) -> String {
    t.map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

impl Session {
    pub fn new(vault: Vault) -> Self {
        let mut session = Session {
            vault,
            error: None,
            allow_close: false,
            recovery: None,
            lock_notice: None,
            security: Some(crate::hardening::security_status()),
            // Everything active, regardless of which group it lives in.
            view: View::All,
            sort: Sort::Title,
            columns: crate::settings::columns(),
            table_sort: crate::settings::table_sort(),
            selected: None,
            search: String::new(),
            search_opts: SearchOptions::default(),
            reveal: false,
            reveal_previous: false,
            reveal_notes: false,
            reveal_fields: false,
            editor: None,
            confirm: None,
            passkey_job: None,
            rotation_job: None,
            key_change: None,
            breach_job: None,
            breached: None,
            unlock_keys: Vec::new(),
            last_activity: Instant::now(),
            notice: None,
            categories: HashMap::new(),
            categories_at: None,
            page: Page::Vault,
            tag_filter: None,
            tag_input: None,
            show_security: true,
            activity: Vec::new(),
            generator: GeneratorOptions::default(),
            generated: generator::generate(&GeneratorOptions::default()),
            icon_job: None,
            icon_cache: HashMap::new(),
            backups: Default::default(),
        };
        session.load_unlock_keys();
        session
    }

    /// Right after unlock: offers to move old copies beside the database into the backup folder,
    /// and shows the most important message: changes recovered after a failed save, a key
    /// changed outside Axentra, a security warning, or a backup that couldn't go where backups
    /// go.
    pub fn after_unlock(&mut self, recovered: bool) {
        self.offer_migration();
        self.error = if recovered {
            Some("Recovered unsaved changes. The previous save failed; review the database file before saving again.".into())
        } else if self.vault.outside_change() {
            Some(OUTSIDE_CHANGE.into())
        } else if let Some(warning) = self.security.and_then(|s| s.warning()) {
            Some(warning.into())
        } else {
            // The `opened` copy couldn't go where backups normally go.
            self.vault.take_backup_notice()
        };
    }

    /// The entry's KeePass custom icon as an image source, if it has one.
    fn entry_icon(&mut self, id: EntryId) -> Option<egui::ImageSource<'static>> {
        let e = self.vault.db.entry(id)?;
        let icon = e.custom_icon()?;
        let uuid = icon.id().uuid();
        let bytes = self
            .icon_cache
            .entry(uuid)
            .or_insert_with(|| std::sync::Arc::from(icon.data.clone().into_boxed_slice()))
            .clone();
        Some(egui::ImageSource::Bytes {
            uri: format!("bytes://keepass-icon/{uuid}.png").into(),
            bytes: egui::load::Bytes::Shared(bytes),
        })
    }

    /// Entries a download would cover, grouped by host.
    fn icon_targets(&self, only: Option<EntryId>) -> HashMap<String, Vec<EntryId>> {
        let mut by_host: HashMap<String, Vec<EntryId>> = HashMap::new();
        for e in self.vault.db.iter_all_entries() {
            let wanted = match only {
                Some(id) => e.id() == id,
                None => !organize::in_recycle_bin(&e) && e.custom_icon().is_none(),
            };
            if wanted && let Some(host) = crate::siteicons::icon_host(e.get_url().unwrap_or("")) {
                by_host.entry(host).or_default().push(e.id());
            }
        }
        by_host
    }

    /// Starts downloading site icons in the background (contacts each site directly).
    fn start_icon_download(&mut self, ctx: &egui::Context, only: Option<EntryId>) {
        if self.icon_job.is_some() {
            return;
        }
        let hosts: Vec<String> = self.icon_targets(only).into_keys().collect();
        if hosts.is_empty() {
            self.notify(if only.is_some() {
                "This entry has no web address to fetch an icon from."
            } else {
                "Every entry with a web address already has an icon."
            });
            return;
        }
        let progress = std::sync::Arc::new(crate::siteicons::Progress::default());
        let p = progress.clone();
        self.log(
            Icon::Download,
            format!("Fetching icons from {} site(s)", hosts.len()),
        );
        self.icon_job = Some(IconJob {
            job: Job::spawn(ctx, move || crate::siteicons::fetch_all(hosts, p)),
            progress,
            only,
        });
    }

    /// Applies finished downloads: one KeePass custom icon per distinct image, shared by every
    /// entry on that site.
    fn poll_icon_job(&mut self) {
        let Some(result) = self.icon_job.as_ref().and_then(|j| j.job.poll()) else {
            return;
        };
        let only = self.icon_job.take().and_then(|j| j.only);
        match result {
            Ok(result) => self.apply_icon_results(result, only),
            Err(jobs::WorkerDied) => {
                self.error = Some("Icon download stopped unexpectedly; no icons changed.".into())
            }
        }
    }

    fn apply_icon_results(&mut self, result: crate::siteicons::IconResults, only: Option<EntryId>) {
        let targets = self.icon_targets(only);
        let mut existing: HashMap<Vec<u8>, vault_core::keepass::db::CustomIconId> = self
            .vault
            .db
            .iter_all_custom_icons()
            .map(|i| (i.data.clone(), i.id()))
            .collect();
        let (mut entries, mut sites, mut failed) = (0, 0, 0);
        for (host, r) in result {
            let Ok(png) = r else {
                failed += 1;
                continue;
            };
            sites += 1;
            for id in targets.get(&host).into_iter().flatten() {
                let Some(mut e) = self.vault.db.entry_mut(*id) else {
                    continue;
                };
                match existing.get(&png) {
                    Some(icon) => {
                        let _ = e.set_icon_custom(*icon);
                    }
                    None => {
                        let icon = e.set_icon_custom_new(png.clone()).id();
                        existing.insert(png.clone(), icon);
                    }
                }
                entries += 1;
            }
        }
        if entries > 0 {
            self.vault.mark_dirty();
        }
        self.icon_cache.clear();
        let msg = format!(
            "Icons added to {entries} entr{} from {sites} site(s){}. Save to keep them.",
            if entries == 1 { "y" } else { "ies" },
            if failed > 0 {
                format!("; {failed} site(s) had none")
            } else {
                String::new()
            }
        );
        self.log(Icon::Download, msg.clone());
        self.notify(msg);
    }

    fn log(&mut self, icon: Icon, text: impl Into<String>) {
        self.activity.push(Activity {
            at: Instant::now(),
            icon,
            text: text.into(),
        });
        if self.activity.len() > 50 {
            self.activity.remove(0);
        }
    }

    /// Copies a value and records it (by kind and entry title only) in this session's activity.
    fn copy_logged(&mut self, what: &str, title: &str, value: &str) -> SessionAction {
        self.log(Icon::Copy, format!("{what} copied · {title}"));
        SessionAction::Copy(Zeroizing::new(value.to_owned()))
    }

    /// Classifies every entry; cheap enough to redo after each edit and on a slow timer.
    fn refresh_categories(&mut self) {
        if self
            .categories_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(3))
        {
            return;
        }
        self.categories = self
            .vault
            .db
            .iter_all_entries()
            .map(|e| (e.id(), categorize::classify(&e)))
            .collect();
        self.categories_at = Some(Instant::now());
    }

    fn invalidate_categories(&mut self) {
        self.categories_at = None;
    }

    fn category_label(&self, id: &EntryId) -> &'static str {
        self.categories
            .get(id)
            .map(|(c, _)| c.label())
            .unwrap_or("")
    }

    fn notify(&mut self, msg: impl Into<String>) {
        self.notice = Some((msg.into(), Instant::now()));
        self.error = None;
    }

    /// Locks without asking: saves unsaved changes first, and if saving fails keeps them
    /// encrypted in memory for the next unlock. Used by auto-lock and by Lock requests from the
    /// tray or another program, when nobody may be at the window to answer a prompt.
    ///
    /// A key change re-encrypting backups is cancelled first: this returns `None` and the lock
    /// follows once its worker has stopped (at most a few seconds, see `finish_stopping`).
    pub fn lock_now(&mut self) -> SessionAction {
        if let Some(change) = &mut self.key_change {
            change.stop(AfterCancel::Lock);
            return SessionAction::None;
        }
        // A backup opened read-only is never saved; its changes go with the lock.
        if self.vault.is_read_only() {
            if self.vault.is_dirty() {
                self.lock_notice = Some(
                    "Locked the backup without saving: changes to a backup opened read-only are not kept."
                        .into(),
                );
            }
            return SessionAction::Lock;
        }
        if self.vault.is_dirty()
            && let Err(save_error) = self.vault.save()
        {
            match self.vault.seal() {
                Ok(sealed) => {
                    self.recovery = Some(sealed);
                    self.lock_notice = Some(format!(
                        "Saving failed: {save_error}. Unsaved changes are held encrypted in this process. Unlock and save before closing."
                    ));
                }
                Err(seal_error) => {
                    // A corrupt/unserializable in-memory database must not defeat locking.
                    self.lock_notice = Some(format!(
                        "Locked after saving failed: {save_error}. Recovery also failed: {seal_error}. Unsaved changes could not be retained."
                    ));
                }
            }
        }
        SessionAction::Lock
    }

    #[cfg(test)]
    pub(crate) fn expire_idle_timer(&mut self) {
        self.last_activity = Instant::now() - IDLE_LOCK - Duration::from_secs(1);
    }

    /// Test support: a key change's staging has run past its idle-lock ceiling.
    #[cfg(test)]
    pub(crate) fn expire_key_change_ceiling(&mut self) {
        if let Some(change) = &mut self.key_change {
            let past = change.ceiling() + Duration::from_secs(1);
            change.started = Instant::now().checked_sub(past).expect("uptime");
        }
    }

    /// Whether something the user is waiting on holds off the idle lock: a passkey prompt, or a
    /// key change's staging for up to its ceiling (an unattended PC still locks, cancelling it).
    fn idle_lock_held_off(&self) -> bool {
        self.passkey_job.is_some()
            || self.rotation_job.is_some()
            || self
                .key_change
                .as_ref()
                .is_some_and(|k| k.stopping.is_none() && k.started.elapsed() < k.ceiling())
    }

    /// Ends a key change that was asked to stop, once its worker has (or after a few seconds
    /// anyway: a result that arrives later deletes its copies itself). Then stays, locks or
    /// closes, as asked.
    fn finish_stopping(&mut self) -> Option<SessionAction> {
        let change = self.key_change.as_ref()?;
        let (since, after) = change.stopping?;
        // An unpublished result deletes its staged copies when dropped.
        let finished = change.job.poll();
        if finished.is_none() && since.elapsed() < KEY_CHANGE_STOP_WAIT {
            return None;
        }
        drop(finished);
        let cancelled = self.key_change.take().map(|c| c.cancelled());
        let cancelled = cancelled.unwrap_or_else(|| KEY_CHANGE_CANCELLED.into());
        match after {
            AfterCancel::Stay => {
                self.error = Some(cancelled);
                None
            }
            AfterCancel::Lock => {
                let action = self.lock_now();
                self.lock_notice.get_or_insert(cancelled);
                Some(action)
            }
            AfterCancel::Close if self.vault.is_dirty() => {
                self.confirm = Some(Confirm::CloseUnsaved);
                None
            }
            AfterCancel::Close => Some(SessionAction::Close),
        }
    }

    /// Per-frame work shared by both layouts: activity tracking, close and idle-lock handling,
    /// passkey setup, keyboard shortcuts and categories. Returns an action and whether the search
    /// box should take focus (Ctrl+F / Ctrl+K).
    fn frame_logic(&mut self, ctx: &egui::Context) -> (SessionAction, bool) {
        let mut action = SessionAction::None;

        if ctx.input(|i| !i.events.is_empty() || i.pointer.is_moving()) {
            self.last_activity = Instant::now();
        }
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested && !self.allow_close {
            if let Some(change) = &mut self.key_change {
                // Closing would cancel the key change: the dialog asks first.
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                change.close_asked = true;
            } else if self.vault.is_dirty() && !self.vault.is_read_only() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm = Some(Confirm::CloseUnsaved);
            }
        }
        if let Some(action) = self.finish_stopping() {
            return (action, false);
        }
        if self.last_activity.elapsed() >= IDLE_LOCK && !self.idle_lock_held_off() {
            let action = self.lock_now();
            if !matches!(action, SessionAction::None) {
                return (action, false);
            }
        }

        self.poll_passkey_job(ctx);
        self.poll_breach_job();
        self.poll_icon_job();
        self.poll_backups();
        if self.key_change.is_some() {
            // The key change dialog is modal; no shortcut reaches the vault behind it.
            ctx.request_repaint_after(Duration::from_millis(100));
            self.refresh_categories();
            return (action, false);
        }

        // Shortcuts (not while typing in a text field, so Ctrl+C there still copies text).
        let typing = ctx.egui_wants_keyboard_input();
        let (save_key, lock_key, new_key, copy_user, copy_pass, find_key) = ctx.input(|i| {
            let c = i.modifiers.command;
            (
                c && i.key_pressed(Key::S),
                c && i.key_pressed(Key::L),
                c && i.key_pressed(Key::N),
                c && i.key_pressed(Key::B),
                c && i.key_pressed(Key::C),
                c && (i.key_pressed(Key::F) || i.key_pressed(Key::K)),
            )
        });
        if save_key {
            self.save_clicked();
        }
        if lock_key {
            action = self.lock_clicked();
        }
        if new_key && self.editor.is_none() {
            self.start_new_entry();
        }
        if !typing
            && self.editor.is_none()
            && let Some(e) = self.selected.and_then(|id| self.vault.db.entry(id))
        {
            let title = e.get_title().unwrap_or("").to_owned();
            if copy_user {
                let v = e.get_username().unwrap_or("").to_owned();
                action = self.copy_logged("Username", &title, &v);
            } else if copy_pass {
                let v = Zeroizing::new(e.get_password().unwrap_or("").to_owned());
                action = self.copy_logged("Password", &title, &v);
            }
        }
        self.refresh_categories();
        (action, find_key)
    }

    /// The Classic layout: toolbar, sidebar, entry table and details panel.
    pub fn show(&mut self, ui: &mut egui::Ui, clip_left: Option<Duration>) -> SessionAction {
        let ctx = ui.ctx().clone();
        let (mut action, find_key) = self.frame_logic(&ctx);
        if matches!(action, SessionAction::Lock) {
            return action;
        }

        if let Some(a) = self.top_bar(ui, find_key) {
            action = a;
        }
        self.status_bar(ui, clip_left);
        let health = self.health();
        self.sidebar(ui, &health);
        if let Some(a) = self.detail_panel(ui, &health) {
            action = a;
        }
        self.entry_list(ui, &health);
        self.editor_window(&ctx);
        self.backups_dialogs(&ctx);
        if let Some(a) = self.confirm_dialog(&ctx) {
            action = a;
        }
        self.key_change_modal(&ctx);
        action
    }

    // --- Top and status bars ------------------------------------------------------------------

    fn top_bar(&mut self, ui: &mut egui::Ui, focus_search: bool) -> Option<SessionAction> {
        let mut action = None;
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let name = self.vault.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                crate::window_chrome::header(ui, &format!("{}{}", name, if self.vault.is_dirty() { " •" } else { "" }));
            });
            ui.horizontal_wrapped(|ui| {
                if icon::button(ui, Icon::NewEntry, "New entry").on_hover_text("Ctrl+N").clicked() {
                    self.start_new_entry();
                }
                let can_save = self.vault.is_dirty() && !self.vault.is_read_only();
                if ui
                    .add_enabled(can_save, egui::Button::image_and_text(Icon::Save.image(15.0), "Save"))
                    .on_hover_text("Ctrl+S")
                    .on_disabled_hover_text(if self.vault.is_read_only() { READ_ONLY_SAVE } else { "No unsaved changes" })
                    .clicked()
                {
                    self.save_clicked();
                }
                if icon::button(ui, Icon::Lock, "Lock").on_hover_text("Ctrl+L").clicked() {
                    action = Some(self.lock_clicked());
                }
                ui.menu_image_text_button(Icon::Security.image(15.0), "Security", |ui| {
                    let busy = self.key_job_pending();
                    if !self.unlock_keys.is_empty() {
                        ui.label(RichText::new("Unlock keys").strong());
                        let mut remove = None;
                        let removable = self.unlock_keys.len() > 1;
                        for (i, label) in self.unlock_keys.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(label);
                                if ui
                                    .add_enabled(removable && !busy, egui::Button::new("Remove").small())
                                    .on_disabled_hover_text("The only unlock key: add another before removing it.")
                                    .clicked()
                                {
                                    remove = Some(i);
                                }
                            });
                        }
                        if let Some(i) = remove {
                            ui.close();
                            self.remove_unlock_key(i, ui.ctx());
                        }
                        ui.separator();
                    }
                    let (text, hint) = self.passkey_setup_text();
                    if ui.add_enabled(!busy, egui::Button::image_and_text(Icon::Passkey.image(15.0), text)).clicked() {
                        ui.close();
                        self.start_passkey_setup(ui.ctx());
                    }
                    ui.label(RichText::new(hint).weak().small());
                    ui.separator();
                    if ui
                        .add_enabled(self.breach_job.is_none(), egui::Button::image_and_text(Icon::Security.image(15.0), "Check passwords against known breaches…"))
                        .clicked()
                    {
                        ui.close();
                        self.start_breach_check(ui.ctx());
                    }
                    ui.label(RichText::new(self.breach_status()).weak().small());
                });
                ui.menu_image_text_button(Icon::Settings.image(15.0), "Settings", |ui| {
                    ui.label(RichText::new("Layout").strong());
                    if ui.radio(false, "Floating Modular").on_hover_text("Detached glass panels in a frameless window").clicked() {
                        action = Some(SessionAction::SetLayout(crate::settings::Layout::Floating));
                        ui.close();
                    }
                    let _ = ui.radio(true, "Classic");
                    ui.separator();
                    ui.label(RichText::new("Protection").strong());
                    let mut hide = crate::hardening::capture_hidden();
                    if ui
                        .add_enabled(crate::hardening::capture_supported(), egui::Checkbox::new(&mut hide, "Hide from screenshots and screen sharing"))
                        .on_hover_text(crate::hardening::capture_hint())
                        .changed()
                    {
                        crate::hardening::set_capture_hidden(hide);
                        crate::settings::set_hide_from_capture(hide);
                    }
                    if let Some(status) = self.security {
                        ui.label(RichText::new(status.summary()).weak().small());
                    }
                    ui.separator();
                    egui::containers::menu::SubMenuButton::new("Columns")
                        .config(columns::stay_open())
                        .ui(ui, |ui| {
                            let names = self.custom_field_names();
                            if let Some(cols) = columns::menu(ui, &self.columns, &names) {
                                self.set_columns(cols);
                            }
                        });
                    ui.label(RichText::new("Show or hide list columns (also: right-click the column headers)").weak().small());
                    ui.separator();
                    if ui.button("Backups…").clicked() {
                        ui.close();
                        self.backups.dialog = true;
                        self.backups_changed();
                    }
                    ui.label(RichText::new("Where earlier versions are kept; back up now or restore one").weak().small());
                    ui.separator();
                    ui.label(RichText::new("Site icons").strong());
                    match &self.icon_job {
                        Some(job) => {
                            let done = job.progress.done.load(std::sync::atomic::Ordering::Relaxed);
                            let total = job.progress.total.load(std::sync::atomic::Ordering::Relaxed);
                            ui.label(format!("Fetching icons… {done} of {total} sites"));
                        }
                        None => {
                            let missing: usize = self.icon_targets(None).values().map(Vec::len).sum();
                            if ui
                                .add_enabled(missing > 0, egui::Button::image_and_text(Icon::Download.image(14.0), format!("Download icons for {missing} entries")))
                                .on_hover_text("Fetches each site's logo directly from that site and saves it in the vault")
                                .clicked()
                            {
                                self.start_icon_download(ui.ctx(), None);
                                ui.close();
                            }
                        }
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut self.search_opts.include_email_domains, "emails")
                        .on_hover_text("Also match entries whose only link is an email username domain, e.g. @outlook.com");
                    ui.checkbox(&mut self.search_opts.include_archived, "archived");
                    ui.checkbox(&mut self.search_opts.include_notes, "notes");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.search).hint_text("Search (Ctrl+F)").desired_width(240.0),
                    );
                    ui.add(Icon::Search.image(15.0));
                    if focus_search {
                        resp.request_focus();
                    }
                    if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                        self.search.clear();
                    }
                });
            });
            if let Some(banner) = self.read_only_banner() {
                ui.add_space(2.0);
                ui.colored_label(ui.visuals().warn_fg_color, banner);
            }
            ui.add_space(4.0);
        });
        action
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, clip_left: Option<Duration>) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(err) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, err);
                } else if let Some((msg, at)) = &self.notice
                    && at.elapsed() < Duration::from_secs(6)
                {
                    ui.colored_label(Color32::from_rgb(0x3a, 0x9a, 0x5b), msg);
                }
                if let Some(busy) = self.busy_text() {
                    ui.spinner();
                    ui.label(busy);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let idle_left = IDLE_LOCK.saturating_sub(self.last_activity.elapsed());
                    ui.label(
                        RichText::new(format!(
                            "Auto-lock in {}:{:02}",
                            idle_left.as_secs() / 60,
                            idle_left.as_secs() % 60
                        ))
                        .weak(),
                    );
                    if let Some(left) = clip_left {
                        ui.label(
                            RichText::new(format!("Clipboard clears in {}s", left.as_secs() + 1))
                                .weak(),
                        );
                    }
                });
            });
        });
    }

    // --- Sidebar: views and groups ------------------------------------------------------------

    fn sidebar(&mut self, ui: &mut egui::Ui, health: &HashMap<EntryId, Vec<Issue>>) {
        let tree = build_tree(self.vault.db.root());
        let bin = self.vault.db.recycle_bin().map(|g| g.id());
        let archived = self
            .vault
            .db
            .iter_all_entries()
            .filter(|e| organize::is_archived(e) && !organize::in_recycle_bin(e))
            .count();
        let searching = !self.search.trim().is_empty();

        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(230.0)
            .size_range(160.0..=380.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.label(RichText::new("VIEWS").weak().small());
                        for (view, glyph, label) in [
                            (View::All, Icon::Vault, "All entries".to_owned()),
                            (View::Recent, Icon::History, "Recently changed".to_owned()),
                            (
                                View::Attention,
                                Icon::Attention,
                                format!("Needs attention  ({})", health.len()),
                            ),
                            (
                                View::Archived,
                                Icon::Archive,
                                format!("Archived  ({archived})"),
                            ),
                        ] {
                            let row = egui::Button::selectable(
                                !searching && self.view == view,
                                (glyph.image(15.0), label),
                            );
                            if ui.add(row).clicked() {
                                self.set_view(view);
                            }
                        }
                        ui.add_space(8.0);
                        ui.label(RichText::new("CATEGORIES").weak().small());
                        self.category_tree(ui, searching);
                        ui.add_space(8.0);
                        ui.label(RichText::new("GROUPS").weak().small());
                        self.group_node(ui, &tree, bin, true, searching);
                    });
            });
    }

    /// Areas (Financial, Personal, ...) with their categories and live counts of active entries.
    fn category_tree(&mut self, ui: &mut egui::Ui, searching: bool) {
        let mut counts: HashMap<Category, usize> = HashMap::new();
        for e in self.vault.db.iter_all_entries() {
            if organize::is_archived(&e) || organize::in_recycle_bin(&e) {
                continue;
            }
            if let Some((c, _)) = self.categories.get(&e.id()) {
                *counts.entry(*c).or_default() += 1;
            }
        }
        for area in Area::ALL {
            let cats: Vec<(Category, usize)> = Category::ALL
                .into_iter()
                .filter(|c| c.area() == area)
                .filter_map(|c| counts.get(&c).map(|n| (c, *n)))
                .collect();
            let total: usize = cats.iter().map(|(_, n)| n).sum();
            if total == 0 {
                continue;
            }
            let selected = !searching && self.view == View::Area(area);
            let id = ui.make_persistent_id(("area", area.label()));
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                true,
            );
            if selected {
                state.set_open(true);
            }
            state
                .show_header(ui, |ui| {
                    if ui
                        .add(egui::Button::selectable(
                            selected,
                            format!("{}  ({total})", area.label()),
                        ))
                        .clicked()
                    {
                        self.set_view(View::Area(area));
                    }
                })
                .body(|ui| {
                    for (c, n) in cats {
                        let selected = !searching && self.view == View::Category(c);
                        if ui
                            .add(egui::Button::selectable(
                                selected,
                                format!("{}  ({n})", c.label()),
                            ))
                            .clicked()
                        {
                            self.set_view(View::Category(c));
                        }
                    }
                });
        }
    }

    fn group_node(
        &mut self,
        ui: &mut egui::Ui,
        node: &GroupNode,
        bin: Option<GroupId>,
        root: bool,
        searching: bool,
    ) {
        let is_bin = Some(node.id) == bin;
        let text = if node.active > 0 {
            format!("{}  ({})", node.name, node.active)
        } else {
            node.name.clone()
        };
        let selected = !searching && self.view == View::Group(node.id);
        let row = |selected: bool, text: String| -> egui::Button<'static> {
            if is_bin {
                egui::Button::selectable(selected, (Icon::RecycleBin.image(14.0), text))
            } else {
                egui::Button::selectable(selected, text)
            }
        };
        if node.children.is_empty() {
            if ui.add(row(selected, text)).clicked() {
                self.set_view(View::Group(node.id));
            }
            return;
        }
        let id = ui.make_persistent_id(node.id.uuid());
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, root)
            .show_header(ui, |ui| {
                if ui.add(row(selected, text)).clicked() {
                    self.set_view(View::Group(node.id));
                }
            })
            .body(|ui| {
                for child in &node.children {
                    self.group_node(ui, child, bin, false, searching);
                }
            });
    }

    fn set_view(&mut self, view: View) {
        self.view = view;
        self.search.clear();
        self.selected = None;
        if view == View::Recent {
            self.sort = Sort::Modified;
            self.table_sort = (Some(Col::Modified), true);
        }
    }

    // --- Entry list ---------------------------------------------------------------------------

    fn set_columns(&mut self, cols: Vec<Col>) {
        crate::settings::set_columns(&cols);
        self.columns = cols;
    }

    /// Custom (string) field names used in this vault, most common first (max 40).
    fn custom_field_names(&self) -> Vec<String> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for e in self.vault.db.iter_all_entries() {
            for name in e.fields.keys() {
                if !fields::KNOWN_FIELDS.contains(&name.as_str()) {
                    *counts.entry(name.clone()).or_default() += 1;
                }
            }
        }
        let mut names: Vec<(String, usize)> = counts.into_iter().collect();
        names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut names: Vec<String> = names.into_iter().take(40).map(|(n, _)| n).collect();
        names.sort_by_key(|n| n.to_lowercase());
        names
    }

    fn row(&self, e: &Entry, note: String, dim: bool) -> Row {
        let modified = e.times.last_modification;
        let masked = || "••••••".to_owned();
        let mut extra = HashMap::new();
        for col in &self.columns {
            let text = match col {
                Col::Group => self
                    .vault
                    .db
                    .entry(e.id())
                    .map(|r| r.parent().name.clone())
                    .unwrap_or_default(),
                Col::Tags => organize::user_tags(e)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                Col::Created => fmt_date(e.times.creation),
                Col::Expires => match (e.times.expires, e.times.expiry) {
                    (Some(true), Some(t)) if t < chrono::Utc::now().naive_utc() => {
                        format!("{} (expired)", fmt_date(Some(t)))
                    }
                    (Some(true), t) => fmt_date(t),
                    _ => String::new(),
                },
                Col::Notes => match e.fields.get(fields::NOTES) {
                    Some(v) if v.is_protected() => masked(),
                    Some(v) => v
                        .get()
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(160)
                        .collect(),
                    None => String::new(),
                },
                Col::Custom(name) => match e.fields.get(name.as_str()) {
                    Some(v) if v.is_protected() => masked(),
                    Some(v) => v.get().chars().take(160).collect(),
                    None => String::new(),
                },
                _ => continue,
            };
            extra.insert(col.clone(), text);
        }
        Row {
            extra,
            category: self.category_label(&e.id()),
            id: e.id(),
            title: e.get_title().unwrap_or("").to_owned(),
            username: e.get_username().unwrap_or("").to_owned(),
            host: search::host_of(e.get_url().unwrap_or("")).to_owned(),
            modified: modified.map(|t| t.and_utc().timestamp()),
            modified_text: fmt_date(modified),
            note,
            dim,
        }
    }

    /// Rows for the current search or view, plus how many archived entries the view hides.
    fn rows(&self, health: &HashMap<EntryId, Vec<Issue>>) -> (Vec<Row>, usize, bool) {
        let db = &self.vault.db;
        if !self.search.trim().is_empty() {
            let hits = search::search(db, &self.search, self.search_opts);
            let rows = hits
                .iter()
                .filter_map(|h| {
                    let e = db.entry(h.id)?;
                    let fields: Vec<&str> = h.matched.iter().map(|f| f.label()).collect();
                    let archived = organize::is_archived(&e);
                    let note = format!(
                        "{}{}",
                        if archived { "archived · " } else { "" },
                        fields.join(", ")
                    );
                    Some(self.row(&e, note, archived))
                })
                .collect();
            return (rows, 0, true); // already ranked
        }

        let issues = |id: &EntryId| {
            health
                .get(id)
                .map(|v| v.iter().map(|i| i.label()).collect::<Vec<_>>().join(", "))
                .unwrap_or_default()
        };
        let active = |e: &vault_core::keepass::db::EntryRef<'_>| {
            !organize::is_archived(e) && !organize::in_recycle_bin(e)
        };
        let now = Times::now();
        let mut hidden = 0;
        let rows: Vec<Row> = match self.view {
            View::All => db
                .iter_all_entries()
                .filter(active)
                .map(|e| self.row(&e, issues(&e.id()), false))
                .collect(),
            View::Recent => db
                .iter_all_entries()
                .filter(active)
                .filter(|e| {
                    e.times
                        .last_modification
                        .is_some_and(|t| (now - t).num_days() <= RECENT_DAYS)
                })
                .map(|e| self.row(&e, issues(&e.id()), false))
                .collect(),
            View::Attention => db
                .iter_all_entries()
                .filter(|e| health.contains_key(&e.id()))
                .map(|e| self.row(&e, issues(&e.id()), false))
                .collect(),
            View::Archived => db
                .iter_all_entries()
                .filter(|e| organize::is_archived(e) && !organize::in_recycle_bin(e))
                .map(|e| {
                    let when = organize::archived_at(&e)
                        .map(|t| format!("archived {}", t.format("%Y-%m-%d")))
                        .unwrap_or_else(|| "archived".into());
                    self.row(&e, when, true)
                })
                .collect(),
            View::Area(area) => db
                .iter_all_entries()
                .filter(active)
                .filter(|e| {
                    self.categories
                        .get(&e.id())
                        .is_some_and(|(c, _)| c.area() == area)
                })
                .map(|e| self.row(&e, issues(&e.id()), false))
                .collect(),
            View::Category(cat) => db
                .iter_all_entries()
                .filter(active)
                .filter(|e| self.categories.get(&e.id()).is_some_and(|(c, _)| *c == cat))
                .map(|e| self.row(&e, issues(&e.id()), false))
                .collect(),
            View::Group(gid) => {
                let in_bin = db.recycle_bin().is_some_and(|b| b.id() == gid);
                db.group(gid)
                    .map(|g| {
                        g.entries()
                            .filter(|e| {
                                let keep = in_bin || !organize::is_archived(e);
                                hidden += usize::from(!keep);
                                keep
                            })
                            .map(|e| self.row(&e, issues(&e.id()), in_bin))
                            .collect()
                    })
                    .unwrap_or_default()
            }
        };
        (rows, hidden, false)
    }

    fn entry_list(&mut self, ui: &mut egui::Ui, health: &HashMap<EntryId, Vec<Issue>>) {
        let (mut rows, hidden, ranked) = self.rows(health);
        if !ranked {
            sort_rows(&mut rows, self.table_sort.0.as_ref(), self.table_sort.1);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if ranked {
                ui.label(
                    RichText::new(format!("{} result(s), best match first", rows.len()))
                        .weak()
                        .small(),
                );
            }
            if rows.is_empty() {
                ui.add_space(20.0);
                let msg = if ranked {
                    "No matches. Tick “notes” or “archived” to search more widely."
                } else {
                    "Nothing here."
                };
                ui.vertical_centered(|ui| ui.label(RichText::new(msg).weak()));
            } else {
                let cols = self.columns.clone();
                let custom_names = self.custom_field_names();
                let mut new_columns: Option<Vec<Col>> = None;
                let mut sort_changed = false;
                let table_sort = &mut self.table_sort;
                let mut clicked: Option<EntryId> = None;
                let mut opened: Option<EntryId> = None;
                let selected_id = self.selected;
                let available = ui.available_height() - if hidden > 0 { 24.0 } else { 0.0 };
                let mut table = TableBuilder::new(ui)
                    // Widths are remembered per column set.
                    .id_salt(("entries", columns::to_text(&cols)))
                    .striped(true)
                    .resizable(true)
                    .max_scroll_height(available)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::initial(240.0).at_least(120.0).clip(true));
                for (i, col) in cols.iter().enumerate() {
                    table = table.column(if i + 1 == cols.len() {
                        Column::remainder().at_least(40.0).clip(true)
                    } else {
                        Column::initial(col.width()).at_least(50.0).clip(true)
                    });
                }
                if cols.is_empty() {
                    table = table.column(Column::remainder().at_least(40.0).clip(true));
                }
                table
                    .header(24.0, |mut header| {
                        // Click a header to sort by it (again to reverse); right-click anywhere on
                        // the header row to choose columns.
                        let mut header_cell = |ui: &mut egui::Ui, col: Option<&Col>| {
                            let label = col.map_or_else(|| "Title".to_owned(), Col::label);
                            let cell = ui.interact(
                                ui.max_rect(),
                                ui.id().with(("col-header", &label)),
                                egui::Sense::click(),
                            );
                            let active = !ranked && table_sort.0.as_ref() == col;
                            let text = RichText::new(&label).strong();
                            let button = if active {
                                let arrow = if table_sort.1 { Icon::SortDown } else { Icon::SortUp };
                                egui::Button::image_and_text(arrow.image(12.0), text)
                                    .image_tint_follows_text_color(false)
                            } else {
                                egui::Button::new(text)
                            };
                            let resp = ui.add(button.frame(false)).on_hover_text(
                                "Click to sort; click again to reverse. Right-click to add or hide columns.",
                            );
                            if resp.clicked() || cell.clicked() {
                                sort_changed = true;
                                *table_sort = if active {
                                    (col.cloned(), !table_sort.1)
                                } else {
                                    // Dates newest first; text A-Z.
                                    let desc = matches!(
                                        col,
                                        Some(Col::Modified | Col::Created | Col::Expires)
                                    );
                                    (col.cloned(), desc)
                                };
                            }
                            for r in [cell, resp] {
                                egui::Popup::context_menu(&r)
                                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                    .show(|ui| {
                                        if let Some(c) = columns::menu(ui, &cols, &custom_names) {
                                            new_columns = Some(c);
                                        }
                                    });
                            }
                        };
                        header.col(|ui| header_cell(ui, None));
                        for col in &cols {
                            header.col(|ui| header_cell(ui, Some(col)));
                        }
                        if cols.is_empty() {
                            header.col(|_| {});
                        }
                    })
                    .body(|body| {
                        body.rows(26.0, rows.len(), |mut row| {
                            let r = &rows[row.index()];
                            let selected = selected_id == Some(r.id);
                            row.set_selected(selected);
                            row.col(|ui| {
                                let title = if r.title.is_empty() {
                                    "(untitled)"
                                } else {
                                    r.title.as_str()
                                };
                                let text = if r.dim {
                                    RichText::new(title).weak()
                                } else {
                                    RichText::new(title)
                                };
                                let resp =
                                    ui.add(egui::Button::selectable(selected, text).truncate());
                                if resp.clicked() {
                                    clicked = Some(r.id);
                                }
                                if resp.double_clicked() {
                                    opened = Some(r.id);
                                }
                            });
                            for col in &cols {
                                row.col(|ui| {
                                    let text = match col {
                                        Col::Username => RichText::new(&r.username),
                                        Col::Website => RichText::new(&r.host).weak(),
                                        Col::Category => RichText::new(r.category)
                                            .color(crate::theme::CYAN)
                                            .small(),
                                        Col::Modified => RichText::new(&r.modified_text).weak(),
                                        Col::Status => {
                                            RichText::new(&r.note).weak().small().italics()
                                        }
                                        other => RichText::new(
                                            r.extra.get(other).map(String::as_str).unwrap_or(""),
                                        )
                                        .weak(),
                                    };
                                    ui.add(egui::Label::new(text).truncate());
                                });
                            }
                            if cols.is_empty() {
                                row.col(|_| {});
                            }
                            let resp = row.response();
                            if resp.clicked() {
                                clicked = Some(r.id);
                            }
                            if resp.double_clicked() {
                                opened = Some(r.id);
                            }
                        });
                    });
                if let Some(cols) = new_columns {
                    self.set_columns(cols);
                }
                if sort_changed {
                    crate::settings::set_table_sort(self.table_sort.0.as_ref(), self.table_sort.1);
                }
                if let Some(id) = clicked {
                    self.selected = Some(id);
                    self.reveal = false;
                    self.reveal_previous = false;
                    self.reveal_notes = false;
                    self.reveal_fields = false;
                }
                if let Some(id) = opened {
                    self.start_edit(id);
                }
            }
            if hidden > 0 {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "{hidden} archived entr{} hidden here (see Archived).",
                        if hidden == 1 { "y" } else { "ies" }
                    ))
                    .weak()
                    .small(),
                );
            }
        });
    }

    // --- Details ------------------------------------------------------------------------------

    fn detail_panel(
        &mut self,
        ui: &mut egui::Ui,
        health: &HashMap<EntryId, Vec<Issue>>,
    ) -> Option<SessionAction> {
        let mut action = None;
        egui::Panel::right("details")
            .resizable(true)
            .default_size(350.0)
            .size_range(280.0..=560.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if let Some(id) = self.selected {
                            action = self.entry_details(ui, id, health.get(&id));
                        } else {
                            ui.label(RichText::new("Select an entry.").weak());
                        }
                    });
            });
        action
    }

    fn entry_details(
        &mut self,
        ui: &mut egui::Ui,
        id: EntryId,
        issues: Option<&Vec<Issue>>,
    ) -> Option<SessionAction> {
        let Some(e) = self.vault.db.entry(id) else {
            self.selected = None;
            return None;
        };
        let mut action = None;
        let mut toggle_archive = false;
        let small = |ui: &mut egui::Ui, s: &str| {
            ui.label(RichText::new(s).weak().small());
        };

        let archived = organize::is_archived(&e);
        ui.heading(e.get_title().unwrap_or("(untitled)"));
        let mut category_choice: Option<Option<Category>> = None;
        if let Some(&(cat, source)) = self.categories.get(&id) {
            ui.horizontal(|ui| {
                let how = match source {
                    Source::Manual => "set by you",
                    Source::Website => "from website",
                    Source::Title => "from title",
                    Source::Group => "from group",
                    Source::None => "no match",
                };
                egui::ComboBox::from_id_salt(("category", id.uuid()))
                    .selected_text(
                        RichText::new(format!("{} · {}", cat.area().label(), cat.label()))
                            .color(crate::theme::CYAN),
                    )
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(source != Source::Manual, "Automatic")
                            .clicked()
                        {
                            category_choice = Some(None);
                        }
                        ui.separator();
                        for area in Area::ALL {
                            ui.label(RichText::new(area.label()).weak().small());
                            for c in Category::ALL.into_iter().filter(|c| c.area() == area) {
                                if ui
                                    .selectable_label(
                                        source == Source::Manual && c == cat,
                                        c.label(),
                                    )
                                    .clicked()
                                {
                                    category_choice = Some(Some(c));
                                }
                            }
                        }
                    });
                ui.label(RichText::new(how).weak().small());
            });
        }
        if archived {
            let when = organize::archived_at(&e)
                .map(|t| format!(" on {}", t.format("%Y-%m-%d")))
                .unwrap_or_default();
            ui.label(
                RichText::new(format!(
                    "Archived{when}. Kept for reference; hidden from search and lists."
                ))
                .italics()
                .weak(),
            );
        }
        if let Some(issues) = issues {
            let text: Vec<&str> = issues.iter().map(|i| i.label()).collect();
            ui.horizontal(|ui| {
                ui.add(Icon::Attention.image(15.0));
                ui.colored_label(
                    Color32::from_rgb(0xc8, 0x8a, 0x1e),
                    format!("Password {}", text.join(", ")),
                );
            });
        }
        ui.add_space(8.0);

        small(ui, "Username");
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(e.get_username().unwrap_or("—")).truncate());
            if icon::small_button(ui, Icon::Copy, "Copy")
                .on_hover_text("Ctrl+B")
                .clicked()
            {
                action = Some(SessionAction::Copy(Zeroizing::new(
                    e.get_username().unwrap_or("").to_owned(),
                )));
            }
        });
        ui.add_space(6.0);

        small(ui, "Password");
        ui.horizontal(|ui| {
            let pw = e.get_password().unwrap_or("");
            if self.reveal {
                ui.add(egui::Label::new(RichText::new(pw).monospace()).truncate());
            } else {
                ui.label(if pw.is_empty() {
                    "—"
                } else {
                    "••••••••••"
                });
            }
            if reveal_button(ui, self.reveal).clicked() {
                self.reveal = !self.reveal;
            }
            if icon::small_button(ui, Icon::Copy, "Copy")
                .on_hover_text("Ctrl+C")
                .clicked()
            {
                action = Some(SessionAction::Copy(Zeroizing::new(pw.to_owned())));
            }
        });
        if let Some(set) = organize::password_set_at(&e) {
            small(ui, &format!("Set {}", set.format("%Y-%m-%d")));
        }
        ui.add_space(6.0);

        small(ui, "URL");
        ui.horizontal(|ui| {
            let url = e.get_url().unwrap_or("");
            ui.add(egui::Label::new(if url.is_empty() { "—" } else { url }).truncate());
            if !url.is_empty() && icon::small_button(ui, Icon::Copy, "Copy").clicked() {
                action = Some(SessionAction::Copy(Zeroizing::new(url.to_owned())));
            }
        });
        ui.add_space(6.0);
        let notes = e.get(fields::NOTES).unwrap_or("");
        let notes_protected = e
            .fields
            .get(fields::NOTES)
            .is_some_and(|v| v.is_protected());
        ui.horizontal(|ui| {
            small(
                ui,
                if notes_protected {
                    "Notes (protected)"
                } else {
                    "Notes"
                },
            );
            if notes_protected
                && !notes.is_empty()
                && reveal_button(ui, self.reveal_notes).clicked()
            {
                self.reveal_notes = !self.reveal_notes;
            }
        });
        let notes_text = if notes.is_empty() {
            RichText::new("—").weak()
        } else if notes_protected && !self.reveal_notes {
            RichText::new("••••••••••")
        } else {
            RichText::new(notes)
        };
        // Wrap, breaking inside long tokens (keys, secrets) so they never widen the panel.
        ui.add(egui::Label::new(notes_text).wrap());

        let form = EntryForm::from_entry(&e);
        if !form.custom.is_empty() {
            ui.separator();
            let any_protected = form.custom.iter().any(|f| f.protected);
            ui.horizontal(|ui| {
                small(ui, "Fields");
                if any_protected && reveal_button(ui, self.reveal_fields).clicked() {
                    self.reveal_fields = !self.reveal_fields;
                }
            });
            for f in &form.custom {
                small(
                    ui,
                    &if f.protected {
                        format!("{} (protected)", f.name)
                    } else {
                        f.name.clone()
                    },
                );
                ui.horizontal(|ui| {
                    let shown = if f.protected && !self.reveal_fields {
                        "••••••"
                    } else {
                        f.value.as_str()
                    };
                    ui.add(egui::Label::new(shown).truncate());
                    if icon::small_button(ui, Icon::Copy, "Copy").clicked() {
                        action = Some(SessionAction::Copy(f.value.clone()));
                    }
                });
            }
        }

        let previous = organize::previous_passwords(&e);
        if !previous.is_empty() {
            ui.separator();
            egui::CollapsingHeader::new(format!("Previous passwords ({})", previous.len()))
                .id_salt("previous")
                .show(ui, |ui| {
                    ui.checkbox(&mut self.reveal_previous, "Show");
                    for p in &previous {
                        ui.horizontal(|ui| {
                            let text = if self.reveal_previous {
                                RichText::new(&p.password).monospace()
                            } else {
                                RichText::new("••••••••")
                            };
                            ui.add(egui::Label::new(text).truncate());
                            if let Some(t) = p.replaced {
                                ui.label(
                                    RichText::new(format!("until {}", t.format("%Y-%m-%d")))
                                        .weak()
                                        .small(),
                                );
                            }
                            if icon::small_button(ui, Icon::Copy, "Copy").clicked() {
                                action =
                                    Some(SessionAction::Copy(Zeroizing::new(p.password.clone())));
                            }
                        });
                    }
                });
        }

        let attachments: Vec<&str> = e.attachments_named().map(|(n, _)| n).collect();
        if !attachments.is_empty() {
            ui.separator();
            small(ui, "Attachments");
            ui.label(attachments.join(", "));
        }
        let tags: Vec<&String> = e
            .tags
            .iter()
            .filter(|t| !t.eq_ignore_ascii_case(organize::ARCHIVE_TAG))
            .collect();
        if !tags.is_empty() {
            small(ui, "Tags");
            ui.label(
                tags.iter()
                    .map(|t| t.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        if let Some(m) = e.times.last_modification {
            ui.add_space(4.0);
            small(ui, &format!("Modified {}", m.format("%Y-%m-%d %H:%M")));
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if icon::button(ui, Icon::Edit, "Edit").clicked() {
                self.start_edit(id);
            }
            let label = if archived { "Restore" } else { "Archive" };
            if icon::button(
                ui,
                if archived {
                    Icon::Restore
                } else {
                    Icon::Archive
                },
                label,
            )
            .on_hover_text(if archived {
                "Bring this entry back into search and lists."
            } else {
                "For accounts you no longer use: keeps everything, hides it from search and lists."
            })
            .clicked()
            {
                toggle_archive = true;
            }
            if icon::button(ui, Icon::Delete, "Delete").clicked() {
                self.confirm = Some(Confirm::Delete(id));
            }
        });

        if let Some(choice) = category_choice
            && let Some(mut e) = self.vault.db.entry_mut(id)
        {
            categorize::set_manual_category(&mut e, choice);
            self.vault.mark_dirty();
            self.invalidate_categories();
        }
        if toggle_archive && let Some(mut e) = self.vault.db.entry_mut(id) {
            organize::set_archived(&mut e, !archived);
            self.vault.mark_dirty();
            self.notify(if archived {
                "Restored."
            } else {
                "Archived. Find it under Views, Archived."
            });
        }
        action
    }

    // --- Editing ------------------------------------------------------------------------------

    fn current_group(&self) -> GroupId {
        let root = self.vault.db.root().id();
        let bin = self.vault.db.recycle_bin().map(|g| g.id());
        match self.view {
            View::Group(g) if Some(g) != bin => g,
            _ => root,
        }
    }

    fn start_new_entry(&mut self) {
        let generator = GeneratorOptions::default();
        let form = EntryForm::new_with_password(generator::generate(&generator));
        self.editor = Some(Editor {
            id: None,
            group: self.current_group(),
            form,
            reveal: false,
            reveal_notes: false,
            revealed_fields: Default::default(),
            generator,
        });
    }

    fn start_edit(&mut self, id: EntryId) {
        let Some(e) = self.vault.db.entry(id) else {
            return;
        };
        self.editor = Some(Editor {
            id: Some(id),
            group: e.parent().id(),
            form: EntryForm::from_entry(&e),
            reveal: false,
            reveal_notes: false,
            revealed_fields: Default::default(),
            generator: GeneratorOptions::default(),
        });
    }

    /// The entry editor in its own OS window (an embedded window where the platform has none).
    /// Closing it with the title-bar button or Esc cancels the edit.
    fn editor_window(&mut self, ctx: &egui::Context) {
        let Some(ed) = &self.editor else { return };
        let title = match (&ed.id, ed.form.title.trim()) {
            (None, _) => "New entry - Axentra Vault".to_owned(),
            (Some(_), "") => "Edit entry - Axentra Vault".to_owned(),
            (Some(_), t) => format!("{t} - Axentra Vault"),
        };
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([560.0, 720.0])
            .with_min_inner_size([420.0, 520.0])
            .with_icon(crate::icon::app_icon());
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("entry-editor"),
            builder,
            |ui, _class| {
                let cancel =
                    ui.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape));
                egui::CentralPanel::default().show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, true])
                        .show(ui, |ui| self.editor_ui(ui));
                });
                if cancel {
                    self.editor = None;
                }
            },
        );
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui) {
        let tree = build_tree(self.vault.db.root());
        let bin = self.vault.db.recycle_bin().map(|g| g.id());
        let mut groups = Vec::new();
        group_paths(&tree, "", bin, &mut groups);

        let Some(ed) = self.editor.as_mut() else {
            return;
        };
        let mut done: Option<bool> = None; // Some(true) = apply, Some(false) = cancel

        ui.heading(if ed.id.is_some() {
            "Edit entry"
        } else {
            "New entry"
        });
        ui.add_space(8.0);
        let full = |ui: &mut egui::Ui, label: &str, text: &mut String| {
            ui.label(RichText::new(label).weak().small());
            // Not secrets: keep normal copy/undo. Undo stacks are wiped on lock with the rest of
            // the widget state; passwords, protected notes and fields use `secret_edit`.
            ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
            ui.add_space(6.0);
        };
        full(ui, "Title", &mut ed.form.title);

        ui.label(RichText::new("Group").weak().small());
        let current = groups
            .iter()
            .find(|(g, _)| *g == ed.group)
            .map(|(_, p)| p.clone())
            .unwrap_or_default();
        egui::ComboBox::from_id_salt("group")
            .selected_text(current)
            .width((ui.available_width() - 16.0).max(80.0))
            .truncate()
            .show_ui(ui, |ui| {
                for (g, path) in &groups {
                    ui.selectable_value(&mut ed.group, *g, path);
                }
            });
        ui.add_space(6.0);

        full(ui, "Username", &mut ed.form.username);
        ui.label(RichText::new("Password").weak().small());
        ui.horizontal(|ui| {
            // Buttons first from the right; the field then takes exactly the remaining width.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon::small_button(ui, Icon::Generate, "Generate").clicked() {
                    ed.form.password = generator::generate(&ed.generator);
                }
                if reveal_button(ui, ed.reveal).clicked() {
                    ed.reveal = !ed.reveal;
                }
                crate::secret_edit::show(
                    ui,
                    &mut ed.form.password,
                    ed.reveal,
                    false,
                    ui.available_width(),
                );
            });
        });
        ui.label(
            RichText::new(format!(
                "≈ {:.0} bits",
                organize::estimate_bits(&ed.form.password)
            ))
            .weak()
            .small(),
        );
        ui.collapsing("Generator options", |ui| {
            let g = &mut ed.generator;
            ui.add(egui::Slider::new(&mut g.length, 8..=64).text("length"));
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut g.lowercase, "a-z");
                ui.checkbox(&mut g.uppercase, "A-Z");
                ui.checkbox(&mut g.digits, "0-9");
                ui.checkbox(&mut g.symbols, "!@#");
                ui.checkbox(&mut g.avoid_ambiguous, "no look-alikes");
            });
            ui.label(
                RichText::new(format!("≈ {:.0} bits of entropy", g.entropy_bits()))
                    .weak()
                    .small(),
            );
        });
        ui.add_space(6.0);
        full(ui, "URL", &mut ed.form.url);

        ui.horizontal(|ui| {
            ui.label(RichText::new("Notes").weak().small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ed.form.notes_protected && reveal_button(ui, ed.reveal_notes).clicked() {
                    ed.reveal_notes = !ed.reveal_notes;
                }
                ui.checkbox(&mut ed.form.notes_protected, "Protect notes")
                    .on_hover_text("Hide the notes until you click Show and preserve the KeePass protected-value flag.");
            });
        });
        crate::secret_edit::show(
            ui,
            &mut ed.form.notes,
            !ed.form.notes_protected || ed.reveal_notes,
            true,
            f32::INFINITY,
        );
        ui.add_space(10.0);

        ui.label(RichText::new("Custom fields").weak().small());
        let mut remove: Option<usize> = None;
        for (i, f) in ed.form.custom.iter_mut().enumerate() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon::small_button(ui, Icon::Delete, "Remove").clicked() {
                            remove = Some(i);
                        }
                        ui.checkbox(&mut f.protected, "Protected");
                        ui.add(
                            egui::TextEdit::singleline(&mut f.name)
                                .hint_text("Field name")
                                .desired_width(ui.available_width()),
                        );
                    });
                });
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let shown = ed.revealed_fields.contains(&i);
                        if f.protected && reveal_button(ui, shown).clicked() {
                            if shown {
                                ed.revealed_fields.remove(&i);
                            } else {
                                ed.revealed_fields.insert(i);
                            }
                        }
                        crate::secret_edit::show(
                            ui,
                            &mut f.value,
                            !f.protected || shown,
                            false,
                            ui.available_width(),
                        );
                    });
                });
            });
        }
        if let Some(i) = remove {
            ed.form.custom.remove(i);
            ed.revealed_fields.clear();
        }
        if icon::small_button(ui, Icon::Protected, "Add field")
            .on_hover_text("For PINs, API keys, client secrets and the like.")
            .clicked()
        {
            ed.form.custom.push(CustomField {
                name: String::new(),
                value: Zeroizing::new(String::new()),
                protected: true,
            });
        }
        ui.add_space(10.0);

        let invalid = ed.form.validate().err();
        if let Some(problem) = &invalid {
            ui.colored_label(ui.visuals().error_fg_color, problem);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    invalid.is_none(),
                    egui::Button::image_and_text(
                        Icon::Save.image(15.0),
                        RichText::new("Save entry").strong(),
                    ),
                )
                .clicked()
            {
                done = Some(true);
            }
            if ui.button("Cancel").clicked() {
                done = Some(false);
            }
        });

        match done {
            Some(true) => {
                let ed = self.editor.take().unwrap();
                self.apply_editor(ed);
            }
            Some(false) => self.editor = None,
            None => {}
        }
    }

    fn apply_editor(&mut self, ed: Editor) {
        let db = &mut self.vault.db;
        let mut changed = false;
        match ed.id {
            Some(id) => {
                let Some(e) = db.entry(id) else { return };
                let fields_changed = ed.form.differs_from(&e);
                let moved = e.parent().id() != ed.group;
                if fields_changed {
                    // Keeps the previous version in history, oldest first like KeePass.
                    organize::edit_with_history(db, id, |e| ed.form.apply(e));
                    changed = true;
                }
                if moved
                    && let Some(mut e) = db.entry_mut(id)
                    && e.move_to(ed.group).is_ok()
                {
                    e.times.location_changed = Some(Times::now());
                    changed = true;
                }
                self.selected = Some(id);
            }
            None => {
                let Some(mut g) = db.group_mut(ed.group) else {
                    return;
                };
                let mut e = g.add_entry();
                ed.form.apply(&mut e);
                let now = Times::now();
                e.times.creation = Some(now);
                e.times.last_modification = Some(now);
                e.times.last_access = Some(now);
                e.times.location_changed = Some(now);
                self.selected = Some(e.id());
                self.view = View::Group(ed.group);
                self.search.clear();
                changed = true;
            }
        }
        if changed {
            self.vault.mark_dirty();
            self.invalidate_categories();
        }
    }

    /// Moves to the recycle bin when there is one; deletes permanently from inside it.
    fn delete_entry(&mut self, id: EntryId) {
        let bin = self.vault.db.recycle_bin().map(|g| g.id());
        let Some(mut e) = self.vault.db.entry_mut(id) else {
            return;
        };
        let parent = e.as_ref().parent().id();
        match bin {
            Some(bin) if parent != bin => {
                if e.move_to(bin).is_ok() {
                    e.times.location_changed = Some(Times::now());
                    self.notify("Moved to the recycle bin.");
                }
            }
            _ => {
                e.remove();
                self.vault
                    .db
                    .deleted_objects
                    .insert(id.uuid(), Some(Times::now()));
                self.notify("Entry deleted.");
            }
        }
        self.selected = None;
        self.vault.mark_dirty();
        self.invalidate_categories();
    }

    // --- Save / lock / confirmations ----------------------------------------------------------

    fn save_clicked(&mut self) {
        // Save is off in a backup opened read-only (Ctrl+S included); the banner says why.
        if self.vault.is_read_only() {
            return;
        }
        match self.vault.save() {
            Ok(()) => self.notify_saved("Saved. The previous version is in your backup folder."),
            Err(VaultError::ChangedOnDisk) => self.confirm = Some(Confirm::Overwrite),
            Err(e) => self.error = Some(format!("Save failed: {e}")),
        }
    }

    /// `saved`, or "Saved." with where the previous version went while backups fall back, and
    /// why old backups weren't cleaned up if they weren't.
    fn notify_saved(&mut self, saved: &str) {
        self.backups_changed();
        let fallback = self
            .vault
            .take_backup_notice()
            .or_else(|| self.vault.backup_status().map(str::to_owned));
        let lines: Vec<String> = fallback
            .into_iter()
            .chain(self.vault.backup_warning().map(str::to_owned))
            .collect();
        if lines.is_empty() {
            self.notify(saved);
        } else {
            self.notify(format!("Saved. {}", lines.join(" ")));
        }
    }

    fn lock_clicked(&mut self) -> SessionAction {
        // Nothing to ask about a backup opened read-only: its changes are never saved.
        if self.vault.is_read_only() {
            return self.lock_now();
        }
        if self.vault.is_dirty() {
            self.confirm = Some(Confirm::LockUnsaved);
            SessionAction::None
        } else {
            SessionAction::Lock
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) -> Option<SessionAction> {
        let confirm = self.confirm.as_ref()?;
        let replace_body;
        let (title, body, ok, discard) = match confirm {
            Confirm::Overwrite => (
                "File changed on disk",
                "Another device or app saved this database since you opened it. Saving now replaces those changes (the replaced file is kept as a backup).",
                "Save anyway",
                None,
            ),
            Confirm::ReplaceUnreadable(why) => {
                replace_body = format!(
                    "The vault file on disk isn't a KeePass database Axentra Vault can read ({why}). It may be damaged or only partly synced. Replace it with the version open here? The unreadable file is kept as a backup."
                );
                (
                    "The file on disk can't be read",
                    replace_body.as_str(),
                    "Replace it",
                    None,
                )
            }
            Confirm::LockUnsaved => (
                "Unsaved changes",
                "Save before locking?",
                "Save and lock",
                Some("Lock without saving"),
            ),
            Confirm::CloseUnsaved => (
                "Unsaved changes",
                "Save before closing?",
                "Save and close",
                Some("Close without saving"),
            ),
            Confirm::Delete(_) => (
                "Delete entry",
                "Move this entry to the recycle bin? (From inside the recycle bin, deletion is permanent.) To keep it for reference instead, use Archive.",
                "Delete",
                None,
            ),
        };
        let mut choice: Option<u8> = None; // 0 cancel, 1 ok, 2 discard
        egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.heading(title);
            ui.add_space(6.0);
            ui.label(body);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new(ok).strong()).clicked() {
                    choice = Some(1);
                }
                if let Some(d) = discard
                    && ui.button(d).clicked()
                {
                    choice = Some(2);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(0);
                }
            });
        });

        let choice = choice?;
        let confirm = self.confirm.take()?;
        match (confirm, choice) {
            (_, 0) => None,
            (Confirm::Overwrite, _) => {
                match self.vault.save_overwriting() {
                    Ok(()) => self
                        .notify_saved("Saved over the external change (it is kept as a backup)."),
                    Err(VaultError::UnreadableOnDisk(why)) => {
                        self.confirm = Some(Confirm::ReplaceUnreadable(why.to_string()))
                    }
                    Err(e) => self.error = Some(format!("Save failed: {e}")),
                }
                None
            }
            (Confirm::ReplaceUnreadable(_), _) => {
                match self.vault.save_replacing_unreadable() {
                    Ok(()) => self
                        .notify_saved("Saved over the unreadable file (it is kept as a backup)."),
                    Err(e) => self.error = Some(format!("Save failed: {e}")),
                }
                None
            }
            (Confirm::Delete(id), _) => {
                self.delete_entry(id);
                None
            }
            (Confirm::LockUnsaved, 2) => Some(SessionAction::Lock),
            (Confirm::CloseUnsaved, 2) => Some(SessionAction::Close),
            (c, _) => match self.vault.save() {
                Ok(()) => Some(if matches!(c, Confirm::LockUnsaved) {
                    SessionAction::Lock
                } else {
                    SessionAction::Close
                }),
                Err(VaultError::ChangedOnDisk) => {
                    self.confirm = Some(Confirm::Overwrite);
                    None
                }
                Err(e) => {
                    self.error = Some(format!("Save failed: {e}"));
                    None
                }
            },
        }
    }

    // --- Passkey setup ------------------------------------------------------------------------

    fn start_passkey_setup(&mut self, ctx: &egui::Context) {
        if self.vault.is_read_only() {
            self.error = Some(READ_ONLY_KEYS.into());
            return;
        }
        if self.vault.credentials().password.is_none() {
            self.error = Some(
                "Passkey unlock is added on top of a master password; this database has none."
                    .into(),
            );
            return;
        }
        if self.vault.changed_on_disk() {
            self.error = Some(
                "The database changed on disk; lock and reopen it before setting up a passkey."
                    .into(),
            );
            return;
        }
        let name = self
            .vault
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let hwnd = WindowHandle::foreground();
        let ring = match self.read_ring() {
            Ok(ring) => ring,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        // A database already opened with a key file (a passkey, its backup .keyx or any other key
        // file) gets another key wrapping the 32 bytes KeePass derives from that file: no re-key,
        // and the key file stays valid. The worker first checks that those bytes open the database.
        let existing = self
            .vault
            .credentials()
            .key_file
            .as_deref()
            .and_then(|k| keyfile::raw_key(k))
            .map(|key| ExistingKey {
                key: Zeroizing::new(key),
                prf_input: ring.keys.first().map(|k| k.envelope.prf_input),
                registered: ring
                    .keys
                    .iter()
                    .map(|k| k.envelope.credential_id.clone())
                    .collect(),
                db_path: self.vault.path().to_owned(),
                password: self.vault.credentials().password.clone(),
                kdf_budget: self.vault.kdf_budget(),
            });
        // Checked before any Windows prompt, so no credential is created that couldn't be kept.
        if existing.is_some() && ring.keys.len() >= envelope::MAX_KEYS {
            self.error = Some(ring_full());
            return;
        }
        // First setup changes the database key; adding a key doesn't.
        if existing.is_none() {
            let notes = key_change_notes(&self.vault.key_change_preview());
            let sure = crate::dialogs::message()
                .set_title("Axentra Vault")
                .set_description(format!(
                    "Set up passkey unlock?\n\nRegister a phone passkey or security key, then save a backup key file. The database key will change: afterwards the vault opens with your master password plus the passkey or the backup key file.\n\n{notes}"
                ))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            if sure != rfd::MessageDialogResult::Yes {
                return;
            }
        }
        self.error = None;
        self.passkey_job = Some(Job::spawn(ctx, move || {
            jobs::register_passkey(name, hwnd, existing)
        }));
    }

    /// Button text and hint for setting up passkey unlock or adding another key.
    fn passkey_setup_text(&self) -> (&'static str, &'static str) {
        let wraps_current = self
            .vault
            .credentials()
            .key_file
            .as_deref()
            .and_then(|k| keyfile::raw_key(k))
            .is_some();
        if wraps_current {
            (
                "Add a phone or security key…",
                "Another passkey or YubiKey that also unlocks this vault. Your key file keeps working.",
            )
        } else {
            (
                "Set up passkey unlock…",
                "Adds a phone passkey or FIDO2 security key (YubiKey) as the second factor and saves a backup key file.",
            )
        }
    }

    /// The `.axvault` key ring next to the database; empty when there is none.
    fn read_ring(&self) -> Result<KeyRing, String> {
        let path = envelope::path_for(self.vault.path());
        match envelope::read_text(&path) {
            Ok(text) => KeyRing::parse(&text)
                .map_err(|e| format!("The passkey file {} is unreadable: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(KeyRing::default()),
            Err(e) => Err(format!("Could not read {}: {e}", path.display())),
        }
    }

    pub(crate) fn load_unlock_keys(&mut self) {
        self.unlock_keys = self
            .read_ring()
            .map(|r| r.keys.into_iter().map(|k| k.label).collect())
            .unwrap_or_default();
    }

    /// Password health, plus "found in a data breach" once the user has run the breach check.
    fn health(&self) -> HashMap<EntryId, Vec<Issue>> {
        let mut health = organize::health(&self.vault.db);
        if let Some(breached) = &self.breached
            && !breached.is_empty()
        {
            for e in self.vault.db.iter_all_entries() {
                let pw = e.get_password().unwrap_or("");
                if pw.is_empty() || organize::is_archived(&e) || organize::in_recycle_bin(&e) {
                    continue;
                }
                if breached.contains_key(&crate::breach::sha1_hex(pw)) {
                    health.entry(e.id()).or_default().push(Issue::Breached);
                }
            }
        }
        health
    }

    /// Times this password appears in known breaches, if the check found it.
    fn breach_count(&self, password: &str) -> Option<u64> {
        self.breached
            .as_ref()?
            .get(&crate::breach::sha1_hex(password))
            .copied()
    }

    /// One line: not checked yet, checking, or the result.
    fn breach_status(&self) -> String {
        if let Some((_, progress)) = &self.breach_job {
            use std::sync::atomic::Ordering;
            return format!(
                "Checking… {} of {} ranges",
                progress.done.load(Ordering::Relaxed),
                progress.total.load(Ordering::Relaxed)
            );
        }
        match &self.breached {
            None => "Sends only the first 5 characters of each password's SHA-1 hash to Have I Been Pwned; the match happens on this PC.".into(),
            Some(_) => {
                let n = self
                    .health()
                    .values()
                    .filter(|v| v.contains(&Issue::Breached))
                    .count();
                if n == 0 {
                    "Checked this session: no passwords found in known breaches.".into()
                } else {
                    format!("Checked this session: {n} entr{} use a password found in breaches.", if n == 1 { "y" } else { "ies" })
                }
            }
        }
    }

    fn start_breach_check(&mut self, ctx: &egui::Context) {
        let hashes: Vec<String> = self
            .vault
            .db
            .iter_all_entries()
            .filter(|e| !organize::is_archived(e) && !organize::in_recycle_bin(e))
            .filter_map(|e| {
                e.get_password()
                    .filter(|p| !p.is_empty())
                    .map(crate::breach::sha1_hex)
            })
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        if hashes.is_empty() {
            self.notify("No passwords to check.");
            return;
        }
        let go = crate::dialogs::message()
            .set_title("Axentra Vault")
            .set_description(format!(
                "Check {} password(s) against Have I Been Pwned?\n\nEach password is hashed on this PC and only the first 5 characters of the hash are sent. The service returns hundreds of possible matches and the comparison happens here, so your passwords never leave this PC.",
                hashes.len()
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        if go != rfd::MessageDialogResult::Yes {
            return;
        }
        let progress = std::sync::Arc::new(crate::siteicons::Progress::default());
        let p = progress.clone();
        let job = Job::spawn(ctx, move || crate::breach::check(hashes, p));
        self.breach_job = Some((job, progress));
        self.log(
            Icon::Security,
            "Started a breach check (k-anonymity, hashes only)",
        );
    }

    fn poll_breach_job(&mut self) {
        let Some(result) = self.breach_job.as_ref().and_then(|(j, _)| j.poll()) else {
            return;
        };
        self.breach_job = None;
        match result.unwrap_or_else(|jobs::WorkerDied| {
            Err("The breach check stopped unexpectedly. Try again.".into())
        }) {
            Ok(found) => {
                self.breached = Some(found);
                let status = self.breach_status();
                self.log(Icon::Security, status.clone());
                self.notify(status);
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn key_job_pending(&self) -> bool {
        self.passkey_job.is_some() || self.rotation_job.is_some() || self.key_change.is_some()
    }

    /// What a running key job is doing, for the status line.
    fn busy_text(&self) -> Option<&'static str> {
        if self.key_change.is_some() {
            Some(KEY_CHANGE_BUSY)
        } else if self.key_job_pending() {
            Some(PASSKEY_BUSY)
        } else {
            None
        }
    }

    /// Rotates the database key and reauthorizes each retained authenticator.
    /// The revoked credential itself stays on its authenticator until deleted there.
    fn remove_unlock_key(&mut self, index: usize, ctx: &egui::Context) {
        if self.vault.is_read_only() {
            self.error = Some(READ_ONLY_KEYS.into());
            return;
        }
        let ring = match self.read_ring() {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        if ring.keys.len() < 2 || index >= ring.keys.len() {
            return;
        }
        let label = ring.keys[index].label.clone();
        let notes = key_change_notes(&self.vault.key_change_preview());
        let sure = crate::dialogs::message()
            .set_title("Axentra Vault")
            .set_description(format!(
                "Revoke \"{label}\"?\n\nConfirm each remaining key, then save a NEW backup key file. The database key will change, so older backup key files will no longer open the current vault.\n\n{notes}\n\nCancel any prompt to leave the vault unchanged."
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        if sure != rfd::MessageDialogResult::Yes {
            return;
        }
        let Some(key) = self
            .vault
            .credentials()
            .key_file
            .as_deref()
            .and_then(|k| keyfile::raw_key(k))
        else {
            self.error = Some(
                "Reopen with a passkey or its canonical backup key file before revoking a key."
                    .into(),
            );
            return;
        };
        if self.vault.changed_on_disk() || self.key_job_pending() {
            self.error = Some(
                "Finish the current operation and reopen external changes before revoking a key."
                    .into(),
            );
            return;
        }
        let hwnd = WindowHandle::foreground();
        let current_key = Zeroizing::new(key);
        self.rotation_job = Some(Job::spawn(ctx, move || {
            jobs::rotate_keys(ring, index, current_key, hwnd)
        }));
    }

    /// Applies a finished key job. A worker that died clears its job like any failure, so it
    /// never holds off idle lock; it ran no file changes (those happen here, on success only).
    fn poll_passkey_job(&mut self, ctx: &egui::Context) {
        self.poll_key_change();
        if let Some(result) = self.rotation_job.as_ref().and_then(Job::poll) {
            self.rotation_job = None;
            match result.unwrap_or_else(|jobs::WorkerDied| {
                Err(UnlockError::Failed(
                    "Key revocation stopped unexpectedly; no files changed.".into(),
                ))
            }) {
                Ok(rotated) => {
                    if let Err(error) = self.finish_key_change(ctx, rotated.ring, &rotated.key) {
                        self.error = Some(error);
                    }
                }
                Err(UnlockError::Cancelled) => {
                    self.error = Some("Key revocation cancelled; no files changed.".into())
                }
                Err(error) => self.error = Some(error.into_message()),
            }
            self.load_unlock_keys();
        }
        let Some(result) = self.passkey_job.as_ref().and_then(Job::poll) else {
            return;
        };
        self.passkey_job = None;
        match result.unwrap_or_else(|jobs::WorkerDied| {
            Err(UnlockError::Failed(
                "Passkey setup stopped unexpectedly; no files changed.".into(),
            ))
        }) {
            Ok(new) => {
                let done = if new.added {
                    self.finish_add_key(new)
                } else {
                    self.finish_passkey_setup(ctx, new)
                };
                if let Err(e) = done {
                    self.error = Some(e);
                }
                self.load_unlock_keys();
            }
            Err(UnlockError::Cancelled) => self.error = Some("Passkey setup cancelled.".into()),
            Err(error) => self.error = Some(error.into_message()),
        }
    }

    /// Another key for a vault that already uses a key file: append it to the key ring and keep
    /// a snapshot of the new ring in the store. The database and its key file are untouched.
    fn finish_add_key(&mut self, new: NewPasskey) -> Result<(), String> {
        let mut ring = self.read_ring()?;
        if ring.keys.len() >= envelope::MAX_KEYS {
            return Err(ring_full());
        }
        if ring.find(&new.envelope.credential_id).is_some() {
            return Err("That key is already set up for this vault.".into());
        }
        let label = unique_label(&ring, &new.label);
        ring.keys.push(RingKey {
            label: label.clone(),
            envelope: new.envelope,
        });
        let text = write_ring(&envelope::path_for(self.vault.path()), &ring)
            .map_err(|e| format!("Could not write the passkey file: {e}"))?;
        // The passkey file works either way; a missing snapshot only weakens its recovery.
        let _ = self.vault.snapshot_ring(text.as_bytes());
        self.log(Icon::Passkey, format!("Added unlock key {label}"));
        self.notify(format!(
            "Added {label}. Any of your {} keys unlocks this vault; your key file still works.",
            ring.keys.len()
        ));
        Ok(())
    }

    /// First setup: a fresh key wrapped by the new passkey becomes the database key.
    fn finish_passkey_setup(&mut self, ctx: &egui::Context, new: NewPasskey) -> Result<(), String> {
        let ring = KeyRing {
            keys: vec![RingKey {
                label: new.label,
                envelope: new.envelope,
            }],
        };
        self.finish_key_change(ctx, ring, &new.key)
    }

    /// Asks where the new backup key file goes (a new file, never one of the database's own
    /// files or inside a backup folder), then starts the key change. Order matters: the backup
    /// key file first, then the backups are re-encrypted (a modal job), then the database moves
    /// to the new key, then the passkey file. A failure at any step leaves a way in (old key or
    /// backup key file).
    fn finish_key_change(
        &mut self,
        ctx: &egui::Context,
        ring: KeyRing,
        key: &[u8; 32],
    ) -> Result<(), String> {
        let db_path = self.vault.path().to_owned();
        let stem = db_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "database".into());
        let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let documents = std::env::var_os(home).map(|p| PathBuf::from(p).join("Documents"));
        let config = self.vault.backup_config().cloned();
        let ask = |text: &str| {
            crate::dialogs::message()
                .set_title("Axentra Vault")
                .set_description(text)
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes
        };
        let refuse = |text: &str| {
            crate::dialogs::message()
                .set_title("Axentra Vault")
                .set_description(text)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
        };

        let backup = loop {
            let mut dlg = crate::dialogs::file()
                .set_title("Save backup key file (keep it offline, not in OneDrive)")
                .set_file_name(keyfile::backup_file_name(&stem))
                .add_filter("KeePass key file", &[keyfile::EXTENSION]);
            if let Some(d) = &documents {
                dlg = dlg.set_directory(d);
            }
            let Some(path) = dlg.save_file() else {
                return Err("Key change cancelled: a new backup key file is required. The database and passkey file have not changed.".into());
            };
            match key_file_place(&db_path, config.as_ref(), &path) {
                KeyFilePlace::Refused(why) => {
                    refuse(why);
                    continue;
                }
                KeyFilePlace::Warn(why) if !ask(why) => continue,
                _ => {}
            }
            if in_onedrive(&path)
                && !ask(
                    "That folder syncs to OneDrive alongside your database, which weakens the backup. Save it there anyway?",
                )
            {
                continue;
            }
            break path;
        };
        self.start_key_change(ctx, ring, key, backup)
    }

    /// Writes the new backup key file, then re-encrypts the backups on a worker (see
    /// `vault_core::rekey`) behind a modal dialog; the key change commits when that is done.
    fn start_key_change(
        &mut self,
        ctx: &egui::Context,
        ring: KeyRing,
        key: &[u8; 32],
        backup: PathBuf,
    ) -> Result<(), String> {
        let new = Credentials {
            password: self.vault.credentials().password.clone(),
            key_file: Some(Zeroizing::new(key.to_vec())),
        };
        let plan = self.vault.plan_key_change(new).map_err(|e| {
            format!(
                "Could not start the key change ({e}). The database and passkey file have not changed."
            )
        })?;
        write_new_backup(&backup, key).map_err(|e| {
            format!("Could not create a new backup key file (choose a new filename): {e}")
        })?;
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(rekey::Progress::default());
        let (c, p) = (cancel.clone(), progress.clone());
        let job = Job::spawn(ctx, move || plan.run(&c, &p));
        self.key_change = Some(KeyChange::new(job, cancel, progress, ring, backup));
        Ok(())
    }

    /// Commits a key change whose backups are staged; see `Vault::commit_key_change`.
    fn poll_key_change(&mut self) {
        let Some(change) = &self.key_change else {
            return;
        };
        if change.stopping.is_some() {
            return;
        }
        let Some(result) = change.job.poll() else {
            return;
        };
        let change = self.key_change.take().expect("checked");
        match result {
            Ok(Ok(staged)) => self.commit_key_change(staged, &change),
            Ok(Err(StageError::Cancelled)) => self.error = Some(change.cancelled()),
            Ok(Err(StageError::Io(e))) => {
                let note = change.discard_backup().unwrap_or_default();
                self.error = Some(format!(
                    "Could not re-encrypt the backups ({e}). The key change was cancelled and nothing changed.{note}"
                ))
            }
            Err(jobs::WorkerDied) => {
                let note = change.discard_backup().unwrap_or_default();
                self.error = Some(format!(
                    "The key change stopped unexpectedly; nothing changed.{note}"
                ))
            }
        }
        self.load_unlock_keys();
    }

    fn commit_key_change(&mut self, staged: Staged, change: &KeyChange) {
        self.backups_changed();
        let backup = &change.backup;
        let name = backup.file_name().map(|n| n.to_string_lossy().into_owned());
        match self
            .vault
            .commit_key_change(staged, &change.ring, name.as_deref())
        {
            Err(e) => {
                let note = change.discard_backup().unwrap_or_default();
                self.error = Some(format!(
                    "Could not commit the database credential change ({e}). The original database still uses its old credentials.{note}"
                ))
            }
            Ok(report) => {
                self.log(
                    Icon::Passkey,
                    "Changed the database key and authorized unlock keys",
                );
                if let Some(e) = report.ring_error {
                    self.error = Some(format!(
                        "The database now needs the new key, but the passkey file could not be written ({e}). Open it with your backup key file: {}",
                        backup.display()
                    ));
                } else {
                    let plural = |n: usize| if n == 1 { "backup" } else { "backups" };
                    let mut text = format!(
                        "Unlock keys updated. Use the NEW backup key file: {}. {} {} re-encrypted; {} kept their old credentials (Settings > Backups).",
                        backup.display(),
                        report.rekeyed,
                        plural(report.rekeyed),
                        report.kept_old
                    );
                    if report.held > 0 {
                        text.push_str(&format!(
                            " {} old-key {} in use by another program; Axentra deletes {} at the next unlock.",
                            report.held,
                            if report.held == 1 { "copy is" } else { "copies are" },
                            if report.held == 1 { "it" } else { "them" }
                        ));
                    }
                    for path in &report.left_beside {
                        text.push_str(&format!(
                            " {} beside the vault still has the old credentials (in use by another program); Axentra deletes it at the next unlock.",
                            path.display()
                        ));
                    }
                    if report.held > 0 || !report.left_beside.is_empty() {
                        self.error = Some(text);
                    } else {
                        self.notify(text);
                    }
                }
            }
        }
    }

    /// The key change dialog: progress, and Cancel. Modal, so nothing behind it can be used
    /// while backups are re-encrypted.
    fn key_change_modal(&mut self, ctx: &egui::Context) {
        let Some(change) = &mut self.key_change else {
            return;
        };
        let (done, total) = (change.progress.done(), change.progress.total());
        let (mut cancel, mut close, mut keep_going) = (false, false, false);
        egui::Modal::new(egui::Id::new("key-change")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("Changing the database key");
            ui.add_space(6.0);
            let status = match change.stopping {
                Some((_, AfterCancel::Lock)) => "Locking…".to_owned(),
                Some(_) => "Cancelling…".to_owned(),
                None if total == 0 => "Preparing…".to_owned(),
                None => format!("Re-encrypting backups {done}/{total}…"),
            };
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(status);
            });
            if total > 0 {
                ui.add(egui::ProgressBar::new(done as f32 / total as f32));
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Nothing changes until every backup is ready. Cancel leaves the vault and its backups as they are.",
                )
                .weak()
                .small(),
            );
            ui.add_space(10.0);
            if change.close_asked && change.stopping.is_none() {
                ui.label("Closing the window cancels the key change.");
                ui.horizontal(|ui| {
                    if ui.button("Cancel key change and close").clicked() {
                        close = true;
                    }
                    if ui.button("Keep going").clicked() {
                        keep_going = true;
                    }
                });
            } else if ui
                .add_enabled(change.stopping.is_none(), egui::Button::new("Cancel"))
                .clicked()
            {
                cancel = true;
            }
        });
        if keep_going {
            change.close_asked = false;
        }
        if close {
            change.stop(AfterCancel::Close);
        } else if cancel {
            change.stop(AfterCancel::Stay);
        }
    }
}

/// Where a backup key file may go.
enum KeyFilePlace {
    Fine,
    /// Allowed after the user confirms.
    Warn(&'static str),
    Refused(&'static str),
}

/// Checks a chosen backup key file path: never one of the database's own files (the next save
/// or key change would replace or delete it) or a vault's own backup folder; a warning anywhere
/// else under a backup folder, and on the removable drive that holds the chosen backup folder
/// (whoever has the backups would have the key too).
fn key_file_place(db_path: &Path, config: Option<&BackupConfig>, path: &Path) -> KeyFilePlace {
    use vault_core::backups::{VAULT_FILE, is_within};
    if vault_core::vault::is_database_file(db_path, path) {
        return KeyFilePlace::Refused(
            "That name belongs to the vault's own files. Choose another name for the backup key file.",
        );
    }
    let Some(config) = config else {
        return KeyFilePlace::Fine;
    };
    let roots = config.known_roots();
    let vault_folder = path.parent().is_some_and(|folder| {
        folder.join(VAULT_FILE).is_file()
            && folder.parent().is_some_and(|r| {
                roots
                    .iter()
                    .any(|k| is_within(r, &k.path) && is_within(&k.path, r))
            })
    });
    if vault_folder {
        return KeyFilePlace::Refused(
            "That folder holds your backups. Keep the backup key file somewhere else, ideally offline.",
        );
    }
    if config.root_containing(path).is_some() {
        return KeyFilePlace::Warn(
            "That folder is inside your backup folder: whoever has the backups would have the key too. Save the backup key file there anyway?",
        );
    }
    if let Some(root) = &config.root
        && vault_core::fsops::is_removable(path)
        && vault_core::fsops::same_volume(path, &root.path)
    {
        return KeyFilePlace::Warn(
            "That removable drive also holds your backups: whoever has the drive would have both. Save the backup key file there anyway?",
        );
    }
    KeyFilePlace::Fine
}

/// The lines a key change's confirmation adds about the backups.
fn key_change_notes(preview: &KeyChangePreview) -> String {
    let mut notes = vec![format!(
        "Your {} backups will be re-encrypted to the new key; afterwards they need the new backup key file.",
        preview.backups
    )];
    if !preview.beside.is_empty() {
        let names: Vec<String> = preview
            .beside
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.display().to_string())
            })
            .collect();
        notes.push(format!(
            "{} will be moved into the backup folder and re-encrypted too.",
            names.join(", ")
        ));
    }
    if !preview.unreachable.is_empty() {
        let roots: Vec<String> = preview
            .unreachable
            .iter()
            .map(|r| r.display().to_string())
            .collect();
        notes.push(format!(
            "Backups in {} aren't reachable and will keep the old credentials.",
            roots.join(", ")
        ));
    }
    notes.push(
        "Copies outside the backup folder (OneDrive version history, other apps' backups, phones) keep their old credentials."
            .into(),
    );
    notes.join("\n\n")
}

/// Replaces the key ring beside the database (see `envelope::write_ring`: a new temp file, then
/// a rename that replaces it; no `.old` copy is kept). Returns the text written.
fn write_ring(env_path: &Path, ring: &KeyRing) -> std::io::Result<String> {
    envelope::write_ring(env_path, ring)
}

fn ring_full() -> String {
    format!(
        "This vault already has {} unlock keys, the most a passkey file holds. Revoke one before adding another.",
        envelope::MAX_KEYS
    )
}

/// "Security key (USB)", then "Security key (USB) 2" for a second one of the same kind.
fn unique_label(ring: &KeyRing, base: &str) -> String {
    let taken = |l: &str| ring.keys.iter().any(|k| k.label == l);
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|l| !taken(l))
        .expect("unbounded")
}

/// Sorts table rows by a column (None = Title). Empty values always go last; ties fall back to
/// the title.
fn sort_rows(rows: &mut Vec<Row>, col: Option<&Col>, desc: bool) {
    let key = |r: &Row| -> String {
        match col {
            None => r.title.to_lowercase(),
            Some(Col::Username) => r.username.to_lowercase(),
            Some(Col::Website) => r.host.to_lowercase(),
            Some(Col::Category) => r.category.to_lowercase(),
            // ISO dates sort correctly as text.
            Some(Col::Modified) => r.modified_text.clone(),
            Some(Col::Status) => r.note.to_lowercase(),
            Some(other) => r
                .extra
                .get(other)
                .map(|t| t.to_lowercase())
                .unwrap_or_default(),
        }
    };
    let mut keyed: Vec<(String, String, usize)> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (key(r), r.title.to_lowercase(), i))
        .collect();
    keyed.sort_by(|a, b| {
        use std::cmp::Ordering;
        match (a.0.is_empty(), b.0.is_empty()) {
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => {
                let o = a.0.cmp(&b.0);
                if desc { o.reverse() } else { o }
            }
        }
        .then_with(|| a.1.cmp(&b.1))
    });
    let order: Vec<usize> = keyed.into_iter().map(|k| k.2).collect();
    let mut slots: Vec<Option<Row>> = rows.drain(..).map(Some).collect();
    rows.extend(order.into_iter().filter_map(|i| slots[i].take()));
}

fn in_onedrive(path: &Path) -> bool {
    let mut roots: Vec<PathBuf> = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(|var| std::env::var_os(var).map(PathBuf::from))
        .collect();
    // Linux sets no OneDrive variable; Axentra CloudDrive syncs ~/OneDrive.
    #[cfg(target_os = "linux")]
    roots.extend(std::env::var_os("HOME").map(|home| PathBuf::from(home).join("OneDrive")));
    roots.iter().any(|root| path.starts_with(root))
}

/// Show/Hide toggle with the eye icons.
fn reveal_button(ui: &mut egui::Ui, revealed: bool) -> egui::Response {
    if revealed {
        icon::small_button(ui, Icon::Hide, "Hide")
    } else {
        icon::small_button(ui, Icon::Show, "Show")
    }
}

/// Never overwrite the database, a previous recovery key, or another existing file.
fn write_new_backup(path: &Path, key: &[u8; 32]) -> std::io::Result<()> {
    use std::io::Write;
    let xml = Zeroizing::new(keyfile::build_v2(key));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(xml.as_bytes())?;
    file.sync_all()?;
    let reread = Zeroizing::new(fs::read(path)?);
    if keyfile::raw_key(&reread).as_ref() != Some(key) {
        return Err(std::io::Error::other("backup key verification failed"));
    }
    Ok(())
}

#[cfg(test)]
mod backup_creation_tests {
    use super::*;
    #[test]
    fn replacement_recovery_key_never_overwrites_existing_files() {
        let path = std::env::temp_dir().join(format!("backup-{}.keyx", uuid::Uuid::new_v4()));
        write_new_backup(&path, &[7; 32]).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(write_new_backup(&path, &[8; 32]).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(keyfile::raw_key(&original), Some([7; 32]));
        fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod save_notice_tests {
    use super::*;
    use vault_core::keepass::{Database, config::KdfConfig};
    use vault_core::vault::with_suffix;

    /// The save message says where the previous version went while the backup folder can't be
    /// used, and goes back to normal once it can.
    #[test]
    fn save_reports_a_backup_fallback() {
        let dir = std::env::temp_dir().join(format!("axv-save-notice-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("synthetic.kdbx");
        let credentials = Credentials {
            password: Some(Zeroizing::new("notice-master".into())),
            key_file: None,
        };
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
        db.save(
            &mut fs::File::create(&path).unwrap(),
            credentials.to_key().unwrap(),
        )
        .unwrap();
        // The backup folder can't be created: saves fall back to <db>.bak.
        let root = dir.join("Backups");
        fs::write(&root, b"in the way").unwrap();
        let mut vault = Vault::open(&path, credentials).unwrap();
        vault.attach_store(BackupConfig::new(&root));
        let mut s = Session::new(vault);
        let save = |s: &mut Session| {
            s.vault.mark_dirty();
            s.save_clicked();
            s.notice.take().expect("saved").0
        };
        for _ in 0..2 {
            let message = save(&mut s);
            assert!(
                message.starts_with("Saved. The backup folder")
                    && message.contains("can't be used")
                    && message.contains(".bak")
                    && !message.contains("unavailable:"),
                "{message}"
            );
        }
        fs::remove_file(&root).unwrap();
        assert_eq!(
            save(&mut s),
            "Saved. The previous version is in your backup folder."
        );
        assert!(!with_suffix(&path, ".bak").exists(), "moved into the store");
        drop(s);
        // Backups are read-only.
        for folder in fs::read_dir(&root).unwrap().flatten() {
            for file in fs::read_dir(folder.path()).unwrap().flatten() {
                let _ = vault_core::fsops::set_readonly(&file.path(), false);
            }
        }
        fs::remove_dir_all(dir).ok();
    }

    /// After the master password was changed in KeePass, the first unlock in Axentra says so
    /// (only recovered changes come first), and once a save records the new credentials the
    /// next unlock doesn't.
    #[test]
    fn a_key_changed_outside_axentra_is_reported_after_unlock() {
        crate::ui_tests::isolate_settings();
        let dir = std::env::temp_dir().join(format!("axv-outside-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("synthetic.kdbx");
        let creds = |password: &str| Credentials {
            password: Some(Zeroizing::new(password.into())),
            key_file: None,
        };
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
        db.save(
            &mut fs::File::create(&path).unwrap(),
            creds("before").to_key().unwrap(),
        )
        .unwrap();
        let open = |password: &str| {
            let mut vault = Vault::open(&path, creds(password)).unwrap();
            vault.attach_store(BackupConfig::new(dir.join("Backups")));
            let mut s = Session::new(vault);
            s.security = None;
            s
        };
        let mut s = open("before");
        s.vault.mark_dirty();
        s.vault.save().unwrap();
        s.after_unlock(false);
        assert_eq!(s.error, None);
        drop(s);

        // KeePass changes the master password; everything else, the record included, stays.
        let db =
            Database::parse(&fs::read(&path).unwrap(), creds("before").to_key().unwrap()).unwrap();
        let mut bytes = Vec::new();
        db.save(&mut bytes, creds("changed").to_key().unwrap())
            .unwrap();
        fs::write(&path, bytes).unwrap();

        let mut s = open("changed");
        s.after_unlock(true);
        assert!(s.error.as_deref().unwrap().starts_with("Recovered"));
        s.after_unlock(false);
        assert_eq!(s.error.as_deref(), Some(OUTSIDE_CHANGE));
        s.vault.mark_dirty();
        s.vault.save().unwrap();
        drop(s);
        let mut s = open("changed");
        s.after_unlock(false);
        assert_eq!(s.error, None);
        drop(s);
        for folder in fs::read_dir(dir.join("Backups")).unwrap().flatten() {
            for file in fs::read_dir(folder.path()).unwrap().flatten() {
                let _ = vault_core::fsops::set_readonly(&file.path(), false);
            }
        }
        fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod idle_lock_tests {
    use super::*;
    use vault_core::keepass::{Database, config::KdfConfig};

    #[test]
    fn failed_save_still_locks_and_recovers_only_with_complete_credentials() {
        for missing_file in [false, true] {
            let dir = std::env::temp_dir().join(format!("axv-idle-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join("synthetic.kdbx");
            let credentials = Credentials {
                password: Some(Zeroizing::new("idle-master-canary".into())),
                key_file: Some(Zeroizing::new(vec![63; 32])),
            };
            let mut db = Database::new();
            db.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
            db.save(
                &mut fs::File::create(&path).unwrap(),
                credentials.to_key().unwrap(),
            )
            .unwrap();
            let mut session = Session::new(Vault::open(&path, credentials.clone()).unwrap());
            let entry = {
                let mut root = session.vault.db.root_mut();
                let mut entry = root.add_entry();
                entry.set_protected(fields::PASSWORD, "unsaved-canary");
                entry.id()
            };
            session.vault.mark_dirty();
            if missing_file {
                fs::remove_file(&path).unwrap();
            } else {
                fs::write(&path, b"changed by another writer").unwrap();
            }
            session.last_activity = Instant::now() - IDLE_LOCK - Duration::from_secs(1);
            let (action, _) = session.frame_logic(&egui::Context::default());
            assert!(matches!(action, SessionAction::Lock));
            assert!(session.last_activity.elapsed() >= IDLE_LOCK);
            assert!(session.lock_notice.as_ref().unwrap().contains("encrypted"));
            let sealed = session.recovery.take().unwrap();
            drop(session);
            let mut incomplete = credentials.clone();
            incomplete.key_file = None;
            assert!(Vault::open_sealed(&sealed, incomplete).is_err());
            let mut wrong = credentials.clone();
            wrong.password = Some(Zeroizing::new("wrong".into()));
            assert!(Vault::open_sealed(&sealed, wrong).is_err());
            let mut recovered = Vault::open_sealed(&sealed, credentials).unwrap();
            assert!(recovered.is_dirty());
            assert_eq!(
                recovered.db.entry(entry).unwrap().get_password(),
                Some("unsaved-canary")
            );
            assert!(matches!(recovered.save(), Err(VaultError::ChangedOnDisk)));
            if !missing_file {
                assert_eq!(fs::read(&path).unwrap(), b"changed by another writer");
            }
        }
    }
}

#[cfg(test)]
mod dead_job_tests {
    use super::*;
    use crate::jobs::WorkerDied;
    use vault_core::keepass::{Database, config::KdfConfig};

    const TIMEOUT: Duration = Duration::from_secs(10);
    const CANARY: &str = "dead-job-panic-canary";

    fn session(dir: &Path) -> Session {
        crate::ui_tests::isolate_settings();
        let path = dir.join("synthetic.kdbx");
        let credentials = Credentials {
            password: Some(Zeroizing::new("dead-job-master".into())),
            key_file: None,
        };
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
        db.save(
            &mut fs::File::create(&path).unwrap(),
            credentials.to_key().unwrap(),
        )
        .unwrap();
        Session::new(Vault::open(&path, credentials).unwrap())
    }

    /// The error a dead job left: present, generic, and without the panic text.
    fn assert_generic_error(s: &mut Session) {
        let error = s.error.take().expect("a dead job reports an error");
        assert!(!error.contains(CANARY), "panic text shown: {error}");
    }

    #[test]
    fn panicked_key_jobs_stop_holding_off_idle_lock() {
        let dir = std::env::temp_dir().join(format!("axv-dead-job-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let mut s = session(&dir);
        let ctx = egui::Context::default();
        for rotation in [false, true] {
            s.last_activity = Instant::now();
            let died = if rotation {
                s.rotation_job = Some(Job::spawn(
                    &ctx,
                    || -> Result<jobs::RotatedKeys, UnlockError> { panic!("{CANARY}") },
                ));
                let job = s.rotation_job.as_ref().unwrap();
                matches!(job.wait(TIMEOUT), Some(Err(WorkerDied)))
            } else {
                s.passkey_job = Some(Job::spawn(&ctx, || -> Result<NewPasskey, UnlockError> {
                    panic!("{CANARY}")
                }));
                let job = s.passkey_job.as_ref().unwrap();
                matches!(job.wait(TIMEOUT), Some(Err(WorkerDied)))
            };
            assert!(died, "the worker panicked");
            assert!(s.key_job_pending(), "the slot is still set until polled");

            let (action, _) = s.frame_logic(&ctx);
            assert!(matches!(action, SessionAction::None));
            assert!(
                !s.key_job_pending(),
                "a dead key job no longer counts as pending"
            );
            assert_generic_error(&mut s);

            s.expire_idle_timer();
            let (action, _) = s.frame_logic(&ctx);
            assert!(
                matches!(action, SessionAction::Lock),
                "idle lock runs after a key job died"
            );
        }
        drop(s);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn panicked_breach_and_icon_jobs_clear_their_slots() {
        let dir = std::env::temp_dir().join(format!("axv-dead-job-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let mut s = session(&dir);
        let ctx = egui::Context::default();

        let breach = Job::spawn(&ctx, || -> crate::breach::BreachResult {
            panic!("{CANARY}")
        });
        assert!(matches!(breach.wait(TIMEOUT), Some(Err(WorkerDied))));
        s.breach_job = Some((breach, Default::default()));
        s.frame_logic(&ctx);
        assert!(
            s.breach_job.is_none(),
            "the breach check can be started again"
        );
        assert!(
            s.breached.is_none(),
            "a dead check is never shown as a result"
        );
        assert_generic_error(&mut s);

        let icons = Job::spawn(&ctx, || -> crate::siteicons::IconResults {
            panic!("{CANARY}")
        });
        assert!(matches!(icons.wait(TIMEOUT), Some(Err(WorkerDied))));
        s.icon_job = Some(IconJob {
            job: icons,
            progress: Default::default(),
            only: None,
        });
        s.frame_logic(&ctx);
        assert!(s.icon_job.is_none(), "icon downloads can be started again");
        assert!(!s.vault.is_dirty());
        assert_generic_error(&mut s);
        drop(s);
        fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod breach_tests {
    use super::*;
    use vault_core::keepass::{Database, config::KdfConfig};

    #[test]
    fn breached_passwords_show_in_health_and_follow_edits() {
        let dir = std::env::temp_dir().join(format!("axv-breach-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("synthetic.kdbx");
        let credentials = Credentials {
            password: Some(Zeroizing::new("pw".into())),
            key_file: None,
        };
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
        let (weak, strong) = {
            let mut root = db.root_mut();
            let a = root
                .add_entry()
                .edit(|e| e.set_protected(fields::PASSWORD, "password"))
                .id();
            let b = root
                .add_entry()
                .edit(|e| e.set_protected(fields::PASSWORD, "Xq7#vL2!mR9$tK4p"))
                .id();
            (a, b)
        };
        db.save(
            &mut fs::File::create(&path).unwrap(),
            credentials.to_key().unwrap(),
        )
        .unwrap();
        let mut s = Session::new(Vault::open(&path, credentials).unwrap());
        assert!(!s.health().values().flatten().any(|i| *i == Issue::Breached));

        s.breached = Some(HashMap::from([(
            crate::breach::sha1_hex("password"),
            9_545_824,
        )]));
        assert!(s.health()[&weak].contains(&Issue::Breached));
        assert!(
            !s.health()
                .get(&strong)
                .is_some_and(|v| v.contains(&Issue::Breached))
        );
        assert_eq!(s.breach_count("password"), Some(9_545_824));
        assert!(s.breach_status().contains("1 entry"));

        // Changing the password clears the flag without another check.
        s.vault
            .db
            .entry_mut(weak)
            .unwrap()
            .set_protected(fields::PASSWORD, "Nw4@pZ8!cJ1^rT6y");
        assert!(!s.health().values().flatten().any(|i| *i == Issue::Breached));
        fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod ring_file_tests {
    use super::*;
    use vault_core::envelope::Envelope;
    use vault_core::keepass::{Database, config::KdfConfig};
    use vault_core::limits;

    fn session(dir: &Path) -> Session {
        crate::ui_tests::isolate_settings();
        let path = dir.join("synthetic.kdbx");
        let credentials = Credentials {
            password: Some(Zeroizing::new("ring-file-master".into())),
            key_file: None,
        };
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
        db.save(
            &mut fs::File::create(&path).unwrap(),
            credentials.to_key().unwrap(),
        )
        .unwrap();
        Session::new(Vault::open(&path, credentials).unwrap())
    }

    #[test]
    fn oversized_passkey_file_is_not_read() {
        let dir = std::env::temp_dir().join(format!("axv-ring-file-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let ring = KeyRing {
            keys: vec![RingKey {
                label: "Phone passkey".into(),
                envelope: Envelope::seal(jobs::RP_ID, &[1], [2; 32], &[3; 32], &[4; 32]),
            }],
        }
        .serialize();
        let env_path = envelope::path_for(&dir.join("synthetic.kdbx"));
        fs::write(&env_path, &ring).unwrap();
        let mut s = session(&dir);
        assert_eq!(s.unlock_keys, ["Phone passkey"]);

        let oversized = format!("{ring}{}", "\n".repeat(limits::SIDECAR_MAX as usize));
        fs::write(&env_path, oversized).unwrap();
        let err = s.read_ring().unwrap_err();
        assert!(err.contains("larger than 64 KiB"), "{err}");
        s.load_unlock_keys();
        assert!(s.unlock_keys.is_empty());
        drop(s);
        fs::remove_dir_all(dir).ok();
    }

    fn ring_of(ids: std::ops::RangeInclusive<u8>) -> KeyRing {
        KeyRing {
            keys: ids
                .map(|id| RingKey {
                    label: format!("Key {id}"),
                    envelope: Envelope::seal(jobs::RP_ID, &[id], [2; 32], &[id; 32], &[4; 32]),
                })
                .collect(),
        }
    }

    fn new_key(id: u8) -> NewPasskey {
        NewPasskey {
            envelope: Envelope::seal(jobs::RP_ID, &[id], [2; 32], &[id; 32], &[4; 32]),
            key: Zeroizing::new([4; 32]),
            label: "Security key (USB)".into(),
            added: true,
        }
    }

    /// The passkey file keeps working: no key past the limit, no second copy of a key, and
    /// nothing written that unlocking would refuse.
    #[test]
    fn added_keys_never_make_the_passkey_file_unreadable() {
        let dir = std::env::temp_dir().join(format!("axv-ring-file-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let env_path = envelope::path_for(&dir.join("synthetic.kdbx"));
        let max = envelope::MAX_KEYS as u8;
        fs::write(&env_path, ring_of(1..=max - 1).serialize()).unwrap();
        let mut s = session(&dir);

        let err = s.finish_add_key(new_key(1)).unwrap_err();
        assert!(err.contains("already set up"), "{err}");
        assert!(s.finish_add_key(new_key(max)).is_ok(), "the last free slot");
        assert_eq!(s.read_ring().unwrap().keys.len(), envelope::MAX_KEYS);
        let full = fs::read(&env_path).unwrap();
        let err = s.finish_add_key(new_key(max + 1)).unwrap_err();
        assert!(err.contains("Revoke one"), "{err}");
        assert_eq!(
            fs::read(&env_path).unwrap(),
            full,
            "the full file is unchanged"
        );

        let mut long = ring_of(1..=1);
        long.keys[0].label = "x".repeat(envelope::MAX_LABEL + 1);
        assert!(write_ring(&env_path, &long).is_err());
        assert_eq!(fs::read(&env_path).unwrap(), full);
        drop(s);
        fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod key_change_tests {
    use std::collections::BTreeMap;

    use super::*;
    use vault_core::envelope::Envelope;
    use vault_core::keepass::{Database, config::KdfConfig};

    const TIMEOUT: Duration = Duration::from_secs(20);

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("axv-key-change-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Backups are read-only; clear that before removing the test folder.
    fn clear_and_remove(dir: &Path) {
        fn clear(dir: &Path) {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    clear(&path);
                } else {
                    let _ = vault_core::fsops::set_readonly(&path, false);
                }
            }
        }
        clear(dir);
        let _ = fs::remove_dir_all(dir);
    }

    fn credentials(key: &[u8; 32]) -> Credentials {
        Credentials {
            password: Some(Zeroizing::new("key-change-master".into())),
            key_file: Some(Zeroizing::new(key.to_vec())),
        }
    }

    /// A vault on the password plus the raw key `key`, whose store under `<dir>\Backups` holds
    /// `saves` backups.
    fn session(dir: &Path, key: &[u8; 32], saves: usize) -> Session {
        crate::ui_tests::isolate_settings();
        let path = dir.join("synthetic.kdbx");
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
        db.save(
            &mut fs::File::create(&path).unwrap(),
            credentials(key).to_key().unwrap(),
        )
        .unwrap();
        let mut vault = Vault::open(&path, credentials(key)).unwrap();
        vault.attach_store(BackupConfig::new(dir.join("Backups")).with_free_space(|_| None));
        for i in 0..saves {
            vault
                .db
                .root_mut()
                .add_entry()
                .set_unprotected(fields::TITLE, format!("entry {i}"));
            vault.mark_dirty();
            vault.save().unwrap();
        }
        Session::new(vault)
    }

    /// Keys `ids`, each wrapping `key` (PRF secret: the id repeated).
    fn ring_of(key: &[u8; 32], ids: &[u8]) -> KeyRing {
        KeyRing {
            keys: ids
                .iter()
                .map(|&id| RingKey {
                    label: format!("Key {id}"),
                    envelope: Envelope::seal(jobs::RP_ID, &[id], [2; 32], &[id; 32], key),
                })
                .collect(),
        }
    }

    /// Every file under `dir` and its contents.
    fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_owned()];
        while let Some(d) = stack.pop() {
            for entry in fs::read_dir(&d).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.insert(p.clone(), fs::read(&p).unwrap());
                }
            }
        }
        out
    }

    /// The ring snapshots in the vault's store folders, newest first.
    fn ring_snapshots(s: &Session) -> Vec<vault_core::backups::Backup> {
        let config = s.vault.backup_config().unwrap().clone();
        let (folders, _) = config.existing_folders(
            s.vault.path(),
            &vault_core::backups::vault_id_of(&s.vault.db),
            &mut Default::default(),
        );
        folders
            .iter()
            .flat_map(|f| f.list_rings().unwrap())
            .collect()
    }

    /// A revocation commits only after its backups are staged; at the commit the old rings go:
    /// every ring snapshot of another generation in the store and the `<db>.axvault.old` that
    /// earlier versions kept. Only the new ring is left, beside the database and in the store.
    #[test]
    fn revocation_deletes_old_ring_snapshots_and_legacy_old() {
        let dir = temp_dir();
        let (k1, k2) = ([1u8; 32], [2u8; 32]);
        let mut s = session(&dir, &k1, 2);
        let db = s.vault.path().to_owned();
        let old_ring = ring_of(&k1, &[1, 2, 3]);
        let old_text = envelope::write_ring(&envelope::path_for(&db), &old_ring).unwrap();
        s.vault.snapshot_ring(old_text.as_bytes()).unwrap();
        fs::write(envelope::legacy_old_path_for(&db), &old_text).unwrap();
        assert_eq!(ring_snapshots(&s).len(), 1);

        // Key 1 is revoked: the other two wrap the new key.
        let new_ring = ring_of(&k2, &[2, 3]);
        let ctx = egui::Context::default();
        let backup = dir.join("synthetic-backup.keyx");
        s.start_key_change(&ctx, new_ring.clone(), &k2, backup.clone())
            .unwrap();
        assert!(s.key_job_pending());
        assert_eq!(s.busy_text(), Some(KEY_CHANGE_BUSY));
        let started = Instant::now();
        while s.key_change.is_some() {
            assert!(started.elapsed() < TIMEOUT, "the key change finished");
            std::thread::sleep(Duration::from_millis(10));
            let (action, _) = s.frame_logic(&ctx);
            assert!(matches!(action, SessionAction::None));
        }
        assert!(s.error.is_none(), "{:?}", s.error);
        let notice = s.notice.take().unwrap().0;
        assert!(
            notice.contains(&backup.display().to_string())
                && notice.contains("2 backups re-encrypted; 0 kept their old credentials"),
            "{notice}"
        );

        assert!(!envelope::legacy_old_path_for(&db).exists());
        assert!(!envelope::staged_path_for(&db).exists());
        let current = fs::read_to_string(envelope::path_for(&db)).unwrap();
        assert_eq!(current, new_ring.serialize());
        let rings = ring_snapshots(&s);
        assert_eq!(rings.len(), 1, "only the new ring");
        assert_eq!(fs::read_to_string(&rings[0].path).unwrap(), current);
        assert_eq!(rings[0].generation.as_deref(), Some(s.vault.generation()));
        assert!(Vault::open(&db, credentials(&k1)).is_err());
        assert!(Vault::open(&db, credentials(&k2)).is_ok());
        drop(s);
        clear_and_remove(&dir);
    }

    /// While backups are re-encrypted, a modal dialog covers the vault and the idle lock waits,
    /// up to a ceiling. Past it the lock cancels the key change: the database, the passkey file
    /// and the store are exactly as before, the staged copies gone.
    #[test]
    fn key_change_staging_is_modal_and_lock_cancels_it() {
        let dir = temp_dir();
        let (k1, k2) = ([1u8; 32], [2u8; 32]);
        let mut s = session(&dir, &k1, 3);
        let db = s.vault.path().to_owned();
        envelope::write_ring(&envelope::path_for(&db), &ring_of(&k1, &[1, 2])).unwrap();
        let store = dir.join("Backups");
        let before = (fs::read(&db).unwrap(), files(&store));
        let sidecar = fs::read(envelope::path_for(&db)).unwrap();

        // Staging that has written its copies, then runs on (as a long one would) until it is
        // told to stop.
        let ctx = egui::Context::default();
        let plan = s.vault.plan_key_change(credentials(&k2)).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(rekey::Progress::default());
        let (staged_tx, staged_rx) = std::sync::mpsc::channel();
        let (c, p) = (cancel.clone(), progress.clone());
        let job = Job::spawn(&ctx, move || {
            let staged = plan.run(&c, &p);
            let _ = staged_tx.send(());
            while !c.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            staged
        });
        // The new backup key file, written before staging starts.
        let new_keyx = dir.join("synthetic-backup.keyx");
        write_new_backup(&new_keyx, &k2).unwrap();
        s.key_change = Some(KeyChange::new(
            job,
            cancel,
            progress,
            ring_of(&k2, &[2]),
            new_keyx.clone(),
        ));
        staged_rx.recv_timeout(TIMEOUT).unwrap();
        let staging = files(&store);
        assert!(
            staging
                .keys()
                .any(|p| p.ends_with(vault_core::backups::MANIFEST_FILE)),
            "copies are staged"
        );
        assert!(staging.len() > before.1.len());

        // The dialog is modal over the vault in both layouts.
        for floating in [false, true] {
            for _ in 0..2 {
                ctx.run_ui(Default::default(), |ui| {
                    let _ = if floating {
                        s.show_floating(ui, ui.max_rect(), None)
                    } else {
                        s.show(ui, None)
                    };
                })
                .drop_without_applying_deltas();
            }
            assert_eq!(
                ctx.memory(|m| m.top_modal_layer()).map(|l| l.id),
                Some(egui::Id::new("key-change"))
            );
        }
        assert!(s.key_job_pending());

        // Idle: staging holds the lock off until its ceiling.
        s.expire_idle_timer();
        let (action, _) = s.frame_logic(&ctx);
        assert!(matches!(action, SessionAction::None));
        assert!(s.key_change.is_some(), "still staging");

        // Past the ceiling the lock cancels the key change, waits for it to stop, then locks.
        s.expire_key_change_ceiling();
        let started = Instant::now();
        loop {
            let (action, _) = s.frame_logic(&ctx);
            if matches!(action, SessionAction::Lock) {
                break;
            }
            assert!(started.elapsed() < TIMEOUT, "locked");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(s.key_change.is_none());
        assert_eq!(s.lock_notice.as_deref(), Some(KEY_CHANGE_CANCELLED));
        assert!(
            !new_keyx.exists(),
            "the new backup key file opens nothing: gone"
        );
        assert_eq!(
            fs::read(&db).unwrap(),
            before.0,
            "the database is unchanged"
        );
        assert_eq!(files(&store), before.1, "the store is unchanged");
        assert_eq!(fs::read(envelope::path_for(&db)).unwrap(), sidecar);
        assert!(!envelope::staged_path_for(&db).exists());
        assert!(Vault::open(&db, credentials(&k1)).is_ok());
        drop(s);
        clear_and_remove(&dir);
    }
}

#[cfg(test)]
mod backups_tests {
    #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;

    use super::*;
    use vault_core::backups::Kind;
    use vault_core::keepass::{Database, config::KdfConfig};
    use vault_core::vault::with_suffix;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("axv-backups-ui-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Backups are read-only; clear that before removing the test folder.
    fn clear_and_remove(dir: &Path) {
        fn clear(dir: &Path) {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    clear(&path);
                } else {
                    let _ = vault_core::fsops::set_readonly(&path, false);
                }
            }
        }
        clear(dir);
        let _ = fs::remove_dir_all(dir);
    }

    fn credentials() -> Credentials {
        Credentials {
            password: Some(Zeroizing::new("backups-ui-master".into())),
            key_file: None,
        }
    }

    /// A vault at `<dir>\synthetic.kdbx`, with its store under `<dir>\Backups`.
    fn session(dir: &Path) -> Session {
        crate::ui_tests::isolate_settings();
        let path = dir.join("synthetic.kdbx");
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
        db.save(
            &mut fs::File::create(&path).unwrap(),
            credentials().to_key().unwrap(),
        )
        .unwrap();
        let mut vault = Vault::open(&path, credentials()).unwrap();
        vault.attach_store(BackupConfig::new(dir.join("Backups")).with_free_space(|_| None));
        Session::new(vault)
    }

    fn imported(s: &Session) -> Vec<vault_core::backups::Backup> {
        let config = s.vault.backup_config().unwrap().clone();
        let (folders, _) = config.existing_folders(
            s.vault.path(),
            &vault_core::backups::vault_id_of(&s.vault.db),
            &mut Default::default(),
        );
        folders
            .iter()
            .flat_map(|f| f.list().unwrap())
            .filter(|b| b.name.kind() == Some(Kind::Imported))
            .collect()
    }

    /// After unlock the app offers to move old copies beside the vault into the backup folder.
    /// Each is copied, read back and compared before the original goes; one another program
    /// holds open stays, and the message says so. "Don't ask again" holds for this vault.
    #[cfg(windows)]
    #[test]
    fn migration_copies_verifies_then_deletes() {
        let dir = temp_dir();
        let mut s = session(&dir);
        let db = s.vault.path().to_owned();
        let bak = with_suffix(&db, ".bak");
        fs::copy(&db, &bak).unwrap();
        let bak_bytes = fs::read(&bak).unwrap();
        fs::write(envelope::legacy_old_path_for(&db), "AxentraVault/2\n").unwrap();

        s.offer_migration();
        let offered = s.backups.migration.take().expect("offered after unlock");
        assert_eq!(offered.len(), 2);

        // Another program has the .bak open.
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1) // FILE_SHARE_READ: it can't be deleted meanwhile.
            .open(&bak)
            .unwrap();
        s.migrate_old_files();
        let message = s.error.take().expect("something stayed behind");
        assert!(
            message.contains("Deleted 1 old passkey file.")
                && message.contains(
                    "synthetic.kdbx.bak was copied into the backup folder but is still beside the vault (another program has it open)"
                ),
            "{message}"
        );
        assert!(bak.exists(), "the locked .bak stays");
        assert!(!envelope::legacy_old_path_for(&db).exists());
        let copies = imported(&s);
        assert_eq!(copies.len(), 1);
        assert_eq!(fs::read(&copies[0].path).unwrap(), bak_bytes);

        // Free again: asked again at the next unlock, and this time it goes.
        drop(held);
        s.offer_migration();
        assert!(s.backups.migration.take().is_some());
        s.migrate_old_files();
        assert_eq!(
            s.notice.take().unwrap().0,
            "Moved 1 old copy into the backup folder."
        );
        assert!(!bak.exists());
        assert_eq!(imported(&s).len(), 1, "the copy made before is reused");

        // "Don't ask again".
        fs::copy(&db, &bak).unwrap();
        s.vault.decline_migration();
        s.offer_migration();
        assert!(s.backups.migration.is_none());
        assert!(bak.exists());
        drop(s);
        clear_and_remove(&dir);
    }

    /// "Back up now", then a later save, then Restore from the picker: the vault is saved with
    /// the backup's contents, the version it replaced is kept, and the views start over.
    #[test]
    fn restore_from_the_picker_resaves_the_backup() {
        let dir = temp_dir();
        let mut s = session(&dir);
        s.back_up_now();
        assert!(s.error.is_none(), "{:?}", s.error);
        let id = s
            .vault
            .db
            .root_mut()
            .add_entry()
            .edit(|e| e.set_unprotected(fields::TITLE, "added after the backup"))
            .id();
        s.vault.mark_dirty();
        s.save_clicked();
        s.selected = Some(id);
        let overview = s.vault.backups_task().unwrap().overview();
        let manual = overview
            .backups
            .iter()
            .find(|b| b.kind() == Kind::Manual)
            .expect("the manual backup is listed");
        s.restore_backup(&manual.backup.path, &manual.when());
        let notice = s.notice.take().unwrap().0;
        assert!(notice.starts_with("Restored the backup from"), "{notice}");
        assert!(s.selected.is_none() && !s.vault.is_dirty());
        let reopened = Vault::open(s.vault.path(), credentials()).unwrap();
        assert_eq!(reopened.db.num_entries(), 0, "the backup's contents");
        let after = s.vault.backups_task().unwrap().overview();
        assert!(
            after
                .backups
                .iter()
                .any(|b| b.kind() == Kind::BeforeRestore)
        );
        drop((s, reopened));
        clear_and_remove(&dir);
    }

    /// A backup opened read-only is never saved: an idle lock leaves the file as it was and
    /// keeps no recovery copy of the changes.
    #[test]
    fn read_only_session_never_saves() {
        let dir = temp_dir();
        let path = dir.join("synthetic.kdbx");
        drop(session(&dir));
        let before = fs::read(&path).unwrap();
        let mut vault = Vault::open(&path, credentials()).unwrap();
        vault.set_read_only();
        let mut s = Session::new(vault);
        assert!(s.read_only_banner().is_some());
        s.offer_migration();
        assert!(s.backups.migration.is_none());
        s.vault
            .db
            .root_mut()
            .add_entry()
            .set_unprotected(fields::TITLE, "not kept");
        s.vault.mark_dirty();
        s.save_clicked();
        assert!(s.confirm.is_none() && s.error.is_none());
        s.expire_idle_timer();
        let (action, _) = s.frame_logic(&egui::Context::default());
        assert!(matches!(action, SessionAction::Lock));
        assert!(s.recovery.is_none());
        assert!(s.lock_notice.as_deref().unwrap().contains("without saving"));
        assert_eq!(fs::read(&path).unwrap(), before);
        drop(s);
        clear_and_remove(&dir);
    }
}
