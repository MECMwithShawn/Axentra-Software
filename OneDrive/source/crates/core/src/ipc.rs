//! Session-bus API (`io.github.mecmwithshawn.AxentraOneDrive1`) used by the tray, the
//! flyout, the CLI and the Nautilus extension. Rich results are JSON.

use std::fmt::Display;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::{broadcast::error::RecvError, watch};
use zbus::{fdo, interface, object_server::SignalEmitter};

use crate::config::{normalize, Config, DBUS_NAME, DBUS_PATH};
use crate::db::Activity;
use crate::engine::Engine;
use crate::thumbs::Bucket;

pub struct SyncService {
    engine: Arc<Engine>,
    quit: watch::Sender<bool>,
}

fn failed(e: impl Display) -> fdo::Error {
    fdo::Error::Failed(format!("{e:#}"))
}

/// Run a job that may go through a whole drive off the async threads, so
/// the bus keeps answering meanwhile.
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> fdo::Result<T> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(failed)?
        .map_err(failed)
}

impl SyncService {
    fn ino_for(&self, path: &str) -> fdo::Result<u64> {
        let rel = relative_to(&self.engine.cfg().mount_path(), path).ok_or_else(|| {
            fdo::Error::InvalidArgs(format!("{path} isn't in your OneDrive folder"))
        })?;
        self.engine
            .db
            .with(|s| s.resolve(&rel))
            .map_err(failed)?
            .map(|i| i.ino)
            .ok_or_else(|| fdo::Error::FileNotFound(path.to_string()))
    }
}

/// `path` relative to the OneDrive folder `mount`, with any `.` and `..`
/// worked out first (paths typed on the command line can have them).
fn relative_to(mount: &Path, path: &str) -> Option<String> {
    let path = normalize(Path::new(path));
    let rel = path.strip_prefix(mount).ok()?;
    Some(rel.to_string_lossy().into_owned())
}

/// `paths` as `ItemsChanged` gives them, with `.` and `..` worked out.
fn changed_paths(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .map(|p| normalize(Path::new(p)).display().to_string())
        .collect()
}

/// How many recent activity entries are looked at for each `Changed`, and
/// how many the database keeps (all are looked at when that isn't enough).
const RECENT_ACTIVITY: u32 = 64;
const ACTIVITY_KEPT: u32 = 500;

/// `rel`, a path from the top of the OneDrive folder `mount` ("/Docs/a.txt"),
/// as a path on this computer.
fn in_folder(mount: &Path, rel: &str) -> String {
    normalize(&mount.join(rel.trim_start_matches('/')))
        .display()
        .to_string()
}

/// The items in `recent` (the activity feed, newest first) after `seen`,
/// the newest `seq` looked at before, and those in `rebadged` (see
/// [`Engine::take_rebadged`]), as paths in the OneDrive folder `mount`.
/// Just `mount` when entries since were missed: then anything in it may
/// have changed.
fn changed_items(recent: &[Activity], seen: i64, rebadged: &[String], mount: &Path) -> Vec<String> {
    let new: Vec<&Activity> = recent.iter().take_while(|a| a.seq > seen).collect();
    if new
        .last()
        .is_some_and(|oldest| oldest.seq > seen + 1 && new.len() == recent.len())
    {
        return vec![mount.display().to_string()];
    }
    let mut paths: Vec<String> = new
        .iter()
        .map(|a| a.path.as_str())
        .chain(rebadged.iter().map(String::as_str))
        .map(|rel| in_folder(mount, rel))
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// What has changed since `seen` (see [`changed_items`]), and the newest
/// `seq` in the activity feed now.
fn changed_since(engine: &Engine, seen: i64) -> (Vec<String>, i64) {
    let mut recent = engine.activity(RECENT_ACTIVITY);
    if recent.len() == RECENT_ACTIVITY as usize && recent.last().is_some_and(|a| a.seq > seen) {
        recent = engine.activity(ACTIVITY_KEPT);
    }
    let newest = recent.first().map_or(seen, |a| a.seq);
    let mount = engine.cfg().mount_path();
    let rebadged = engine.take_rebadged();
    (changed_items(&recent, seen, &rebadged, &mount), newest)
}

#[interface(name = "io.github.mecmwithshawn.AxentraOneDrive1")]
impl SyncService {
    /// JSON `Status`.
    async fn status(&self) -> String {
        serde_json::to_string(&self.engine.status()).unwrap_or_default()
    }

    /// JSON `Account`.
    async fn account(&self) -> String {
        serde_json::to_string(&self.engine.account()).unwrap_or_default()
    }

    /// JSON list of recent `Activity`, newest first.
    async fn activity(&self, limit: u32) -> String {
        serde_json::to_string(&self.engine.activity(limit)).unwrap_or_default()
    }

    /// "Always keep on this device".
    async fn pin(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        paths: Vec<String>,
    ) -> fdo::Result<()> {
        for p in &paths {
            let ino = self.ino_for(p)?;
            let engine = self.engine.clone();
            blocking(move || engine.set_pinned(ino, true)).await?;
        }
        let _ = Self::items_changed(&emitter, &changed_paths(&paths)).await;
        Ok(())
    }

    /// Untick "Always keep on this device" (the files stay local for now).
    async fn unpin(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        paths: Vec<String>,
    ) -> fdo::Result<()> {
        for p in &paths {
            let ino = self.ino_for(p)?;
            let engine = self.engine.clone();
            blocking(move || engine.set_pinned(ino, false)).await?;
        }
        let _ = Self::items_changed(&emitter, &changed_paths(&paths)).await;
        Ok(())
    }

    /// "Free up space". Returns how many files had to stay local.
    async fn free_up_space(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        paths: Vec<String>,
    ) -> fdo::Result<u32> {
        let mut kept = 0;
        for p in &paths {
            let ino = self.ino_for(p)?;
            let engine = self.engine.clone();
            kept += blocking(move || engine.free_up_space(ino)).await?;
        }
        let _ = Self::items_changed(&emitter, &changed_paths(&paths)).await;
        Ok(kept)
    }

    async fn share_link(&self, path: String, edit: bool) -> fdo::Result<String> {
        let ino = self.ino_for(&path)?;
        self.engine.share_link(ino, edit).await.map_err(failed)
    }

    async fn web_url(&self, path: String) -> fdo::Result<String> {
        let ino = self.ino_for(&path)?;
        self.engine.web_url(ino).map_err(failed)
    }

    /// JSON list of versions, newest first.
    async fn versions(&self, path: String) -> fdo::Result<String> {
        let ino = self.ino_for(&path)?;
        let v = self.engine.versions(ino).await.map_err(failed)?;
        Ok(serde_json::to_string(&v).unwrap_or_default())
    }

    /// Path of a PNG thumbnail of `path` in the freedesktop thumbnail cache,
    /// or "" if there isn't one. Online-only photos and videos get OneDrive's
    /// thumbnail, without downloading the file. `size` is normal (128 px),
    /// large (256), x-large (512), xx-large (1024) or a pixel size; "" means large.
    async fn get_thumbnail(&self, path: String, size: String) -> fdo::Result<String> {
        let size = Bucket::parse(&size)
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("unknown thumbnail size {size:?}")))?;
        let ino = self.ino_for(&path)?;
        match tokio::time::timeout(Duration::from_secs(20), self.engine.thumbnail(ino, size)).await
        {
            Ok(Ok(p)) => Ok(p.map(|p| p.display().to_string()).unwrap_or_default()),
            Ok(Err(e)) => Err(failed(e)),
            Err(_) => Err(fdo::Error::TimedOut(
                "OneDrive didn't send the thumbnail in time".into(),
            )),
        }
    }

    async fn restore_version(&self, path: String, version_id: String) -> fdo::Result<()> {
        let ino = self.ino_for(&path)?;
        self.engine
            .restore_version(ino, &version_id)
            .await
            .map_err(failed)
    }

    async fn pause(&self, seconds: u32) {
        self.engine.pause(seconds as u64);
    }

    async fn resume(&self) {
        self.engine.resume();
    }

    async fn sync_now(&self) {
        self.engine.wake();
    }

    /// Re-read config.toml and the sign-in tokens. A new `mount_point` takes
    /// effect when the service restarts: until then the folder stays mounted
    /// where it is, and paths are looked up there.
    async fn reload(&self) -> fdo::Result<()> {
        let mut cfg = Config::load(&self.engine.paths.config_file).map_err(failed)?;
        cfg.mount_point = self.engine.cfg().mount_point;
        self.engine.set_config(cfg);
        self.engine.reload_tokens().await;
        Ok(())
    }

    /// Fails unless `drive_id` is the OneDrive this PC syncs (or none is
    /// linked yet). `axentra-onedrive login` asks before it keeps a sign-in.
    async fn check_account(
        &self,
        drive_id: String,
        drive_type: String,
        email: String,
    ) -> fdo::Result<()> {
        self.engine
            .check_account(&drive_id, &drive_type, &email)
            .map_err(failed)
    }

    /// "Unlink this PC".
    async fn sign_out(&self) -> fdo::Result<()> {
        self.engine.sign_out().await.map_err(failed)
    }

    /// Bytes of OneDrive content stored on this device.
    async fn local_usage(&self) -> u64 {
        self.engine.local_usage()
    }

    async fn quit(&self) {
        let _ = self.quit.send(true);
    }

    #[zbus(signal)]
    async fn changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    /// Items whose status may have changed, by path: each one, everything
    /// in it if it's a folder, and so the folder it's in. File managers
    /// look at their badges again without going through every file.
    #[zbus(signal)]
    async fn items_changed(emitter: &SignalEmitter<'_>, paths: &[String]) -> zbus::Result<()>;
}

/// Claim the bus name and forward engine changes as `Changed` signals,
/// with `ItemsChanged` for the items in the activity feed since the last
/// and those whose badge changed besides.
pub async fn serve(engine: Arc<Engine>, quit: watch::Sender<bool>) -> Result<zbus::Connection> {
    let mut changes = engine.subscribe();
    let feed = engine.clone();
    let mut seen = engine.activity(1).first().map_or(0, |a| a.seq);
    // zbus would otherwise let a second daemon take the name away from this one.
    let conn = zbus::connection::Builder::session()?
        .allow_name_replacements(false)
        .replace_existing_names(false)
        .name(DBUS_NAME)?
        .serve_at(DBUS_PATH, SyncService { engine, quit })?
        .build()
        .await?;
    let c = conn.clone();
    tokio::spawn(async move {
        let Ok(iface) = c
            .object_server()
            .interface::<_, SyncService>(DBUS_PATH)
            .await
        else {
            return;
        };
        loop {
            match changes.recv().await {
                Ok(()) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
            // Coalesce bursts (progress updates) to a few signals a second.
            tokio::time::sleep(Duration::from_millis(250)).await;
            while changes.try_recv().is_ok() {}
            let _ = SyncService::changed(iface.signal_emitter()).await;
            let engine = feed.clone();
            let Ok((paths, newest)) =
                tokio::task::spawn_blocking(move || changed_since(&engine, seen)).await
            else {
                continue;
            };
            seen = newest;
            if !paths.is_empty() {
                let _ = SyncService::items_changed(iface.signal_emitter(), &paths).await;
            }
        }
    });
    Ok(conn)
}

/// Client side of the API, for the CLI and the tray.
#[zbus::proxy(
    interface = "io.github.mecmwithshawn.AxentraOneDrive1",
    default_service = "io.github.mecmwithshawn.AxentraOneDrive",
    default_path = "/io/github/mecmwithshawn/AxentraOneDrive",
    gen_blocking = false
)]
pub trait AxentraOneDrive {
    fn status(&self) -> zbus::Result<String>;
    fn account(&self) -> zbus::Result<String>;
    fn activity(&self, limit: u32) -> zbus::Result<String>;
    fn pin(&self, paths: Vec<String>) -> zbus::Result<()>;
    fn unpin(&self, paths: Vec<String>) -> zbus::Result<()>;
    fn free_up_space(&self, paths: Vec<String>) -> zbus::Result<u32>;
    fn share_link(&self, path: String, edit: bool) -> zbus::Result<String>;
    fn web_url(&self, path: String) -> zbus::Result<String>;
    fn versions(&self, path: String) -> zbus::Result<String>;
    fn get_thumbnail(&self, path: String, size: String) -> zbus::Result<String>;
    fn restore_version(&self, path: String, version_id: String) -> zbus::Result<()>;
    fn pause(&self, seconds: u32) -> zbus::Result<()>;
    fn resume(&self) -> zbus::Result<()>;
    fn sync_now(&self) -> zbus::Result<()>;
    fn reload(&self) -> zbus::Result<()>;
    fn check_account(
        &self,
        drive_id: String,
        drive_type: String,
        email: String,
    ) -> zbus::Result<()>;
    fn sign_out(&self) -> zbus::Result<()>;
    fn local_usage(&self) -> zbus::Result<u64>;
    fn quit(&self) -> zbus::Result<()>;
    #[zbus(signal)]
    fn changed(&self) -> zbus::Result<()>;
    #[zbus(signal)]
    fn items_changed(&self, paths: Vec<String>) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::{changed_items, changed_paths, relative_to};
    use crate::db::Activity;
    use std::path::Path;

    #[test]
    fn items_changed_names_what_the_activity_feed_has_since() {
        let mount = Path::new("/home/u/OneDrive");
        let entry = |seq: i64, path: &str| Activity {
            seq,
            ts: 0,
            kind: "downloaded".into(),
            name: String::new(),
            folder: String::new(),
            path: path.into(),
            is_dir: false,
            detail: String::new(),
        };
        // Newest first; a file synced twice is named once.
        let recent = [
            entry(7, "/Photos/b.jpg"),
            entry(6, "/notes.txt"),
            entry(5, "/Photos/b.jpg"),
            entry(4, "/old.txt"),
        ];
        assert_eq!(
            changed_items(&recent, 4, &[], mount),
            [
                "/home/u/OneDrive/Photos/b.jpg",
                "/home/u/OneDrive/notes.txt"
            ]
        );
        // Nothing new: nothing to look at.
        assert!(changed_items(&recent, 7, &[], mount).is_empty());
        assert!(changed_items(&[], 7, &[], mount).is_empty());
        // Entries 2 and 3 weren't fetched (or are gone): anything may have changed.
        assert_eq!(changed_items(&recent, 1, &[], mount), ["/home/u/OneDrive"]);
        // Everything since is there.
        assert_eq!(changed_items(&recent, 3, &[], mount).len(), 3);
    }

    #[test]
    fn items_changed_names_badges_changed_without_activity() {
        let mount = Path::new("/home/u/OneDrive");
        // Shared online: no activity entry, but its badge changed.
        let shared = ["/Docs/s.txt".to_string()];
        assert_eq!(
            changed_items(&[], 0, &shared, mount),
            ["/home/u/OneDrive/Docs/s.txt"]
        );
        // Too many to name: anything in the folder.
        assert_eq!(
            changed_items(&[], 0, &["/".to_string()], mount),
            ["/home/u/OneDrive"]
        );
    }

    #[test]
    fn items_changed_has_the_paths_worked_out() {
        let given = ["/home/u/OneDrive/Photos/../Docs".to_string()];
        assert_eq!(changed_paths(&given), ["/home/u/OneDrive/Docs"]);
    }

    #[test]
    fn paths_are_looked_up_inside_the_onedrive_folder() {
        let mount = Path::new("/home/u/OneDrive");
        let rel = |p: &str| relative_to(mount, p);
        assert_eq!(
            rel("/home/u/OneDrive/Pictures/2024").as_deref(),
            Some("Pictures/2024")
        );
        assert_eq!(
            rel("/home/u/OneDrive/Pictures/2024/..").as_deref(),
            Some("Pictures")
        );
        assert_eq!(rel("/home/u/Downloads/../OneDrive/x").as_deref(), Some("x"));
        assert_eq!(rel("/home/u/OneDrive").as_deref(), Some(""));
        assert_eq!(rel("/home/u/OneDrive/.."), None);
        assert_eq!(rel("/home/u/OneDrive2/x"), None);
        assert_eq!(rel("OneDrive/x"), None);
    }
}
