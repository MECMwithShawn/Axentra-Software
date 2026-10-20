//! Running backups: "Back up now" and the monthly automatic backup.
//!
//! The work (collecting, Argon2id, compression, encryption, writing) runs
//! on its own thread; the UI only waits for the result. One backup runs at
//! a time per process.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration as StdDuration;

use axentra_core::backup::{self, Written, dest};
use axentra_core::{Settings, backup::BackupError};
use axentra_sync::Secret;
use chrono::{DateTime, Local, Utc};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;

use super::vault;
use crate::state::{AppState, store, store_arc, use_app};

/// How often the background host looks whether a backup is due.
const CHECK_EVERY: StdDuration = StdDuration::from_secs(60 * 60);
/// First look after start-up, once the window and sync have settled.
const FIRST_CHECK: StdDuration = StdDuration::from_secs(45);

static BUSY: AtomicBool = AtomicBool::new(false);
/// The preview server's stand-in for the OneDrive folder.
static PREVIEW_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Progress and result of the latest backup in this session.
#[derive(Clone, Debug, PartialEq)]
pub enum RunState {
    Idle,
    Running { manual: bool },
    Finished(Result<Done, String>),
}

/// A finished backup, for the page's result bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub file: String,
    pub size: u64,
    pub folder: String,
    pub in_onedrive: bool,
    pub removed: usize,
}

pub static RUN: GlobalSignal<RunState> = Signal::global(|| RunState::Idle);
/// Bumped whenever backup files or the backup status change.
pub static REV: GlobalSignal<u64> = Signal::global(|| 0);

/// Preview server: the password is kept in memory, and `AXENTRA_BACKUP_DIR`
/// (or a temporary folder) stands in for the OneDrive folder, so the whole
/// page can be tried without the real OneDrive folder ever being touched.
pub fn preview_mode() {
    let dir = std::env::var_os(dest::ENV_OVERRIDE)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::env::temp_dir().join("axentra-preview-backups"));
    let _ = PREVIEW_DIR.set(dir);
    vault::use_memory_keyring();
}

/// Where backups are saved with these settings.
pub fn destination(settings: &Settings) -> dest::Destination {
    let env = match PREVIEW_DIR.get() {
        Some(dir) => dest::DestEnv {
            home: std::env::var_os("HOME").map(PathBuf::from),
            onedrive_mount: Some(dir.clone()),
            ..Default::default()
        },
        None => dest::DestEnv::from_system(),
    };
    dest::resolve(settings.backup_dir.as_deref(), &env)
}

/// When this device last backed up successfully. Files in the folder don't
/// count: another computer sharing the OneDrive folder writes there too,
/// and its backups don't hold this device's data.
pub fn last_success() -> Option<DateTime<Utc>> {
    backup::last_backup(store()).map(|l| l.at)
}

pub fn is_running() -> bool {
    BUSY.load(Ordering::Relaxed)
}

/// Run `f` on a new thread and wait for its result without blocking the UI.
pub async fn off_thread<T: Send + 'static>(name: &str, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
        let _ = tx.send(f());
    });
    if spawned.is_err() {
        return None;
    }
    rx.await.ok()
}

async fn write(target: dest::Destination, password: Secret, keep: u32) -> Result<Written, String> {
    let store = store_arc();
    off_thread("axentra-backup", move || {
        // Never write into the Axentra OneDrive mount point while it isn't
        // mounted: nothing would reach the cloud, and the folder would
        // then stop Axentra OneDrive from mounting.
        target.ready().and_then(|()| backup::create_backup(&store, &target.dir, password.expose(), Utc::now(), keep)).map_err(|e: BackupError| e.to_string())
    })
    .await
    .unwrap_or_else(|| Err("The backup stopped unexpectedly. Try again.".into()))
}

/// Start a backup. Returns `false` when one is already running.
pub fn start(app: AppState, password: Secret, manual: bool) -> bool {
    if BUSY.swap(true, Ordering::SeqCst) {
        return false;
    }
    *RUN.write() = RunState::Running { manual };
    let settings = app.settings.peek().clone();
    let target = destination(&settings);
    spawn_forever(async move {
        let result = write(target.clone(), password, settings.backup_keep).await;
        BUSY.store(false, Ordering::SeqCst);
        let s = store();
        match result {
            Ok(w) => {
                if let Err(e) = backup::record_success(s, &w) {
                    tracing::warn!("backup status: {e}");
                }
                let folder = target.base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| target.base.display().to_string());
                app.toast(if target.in_onedrive { "Backed up to OneDrive".to_string() } else { format!("Backed up to {folder}") });
                *RUN.write() = RunState::Finished(Ok(Done {
                    file: w.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    size: w.size,
                    folder: target.dir.display().to_string(),
                    in_onedrive: target.in_onedrive,
                    removed: w.removed,
                }));
            }
            Err(message) => {
                let toast = backup::record_failure(s, Utc::now(), &message, Local::now().date_naive()).unwrap_or(true);
                if manual || toast {
                    app.error(format!("Backup didn't finish. {message}"));
                }
                *RUN.write() = RunState::Finished(Err(message));
            }
        }
        *REV.write() += 1;
    });
    true
}

/// Start an automatic backup if one is due and the password is available
/// without asking.
async fn check(app: AppState) {
    let settings = app.settings.peek().clone();
    if !settings.backup_auto || is_running() {
        return;
    }
    let last = last_success();
    // Don't touch the keyring unless a backup would be due.
    if !backup::is_due(settings.backup_auto, true, last, Utc::now()) {
        return;
    }
    let Ok(Some(password)) = vault::get(false).await else { return };
    if backup::is_due(settings.backup_auto, !password.is_empty(), last, Utc::now()) {
        start(app, password, false);
    }
}

/// Mounted once at the app root: looks hourly whether the monthly backup
/// is due and runs it. Renders nothing.
#[component]
pub fn BackupHost() -> Element {
    let app = use_app();
    use_future(move || async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            check(app).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
    rsx! {}
}
