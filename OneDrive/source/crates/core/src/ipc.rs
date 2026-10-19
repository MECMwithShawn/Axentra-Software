//! Session-bus API (`io.github.mecmwithshawn.AxentraOneDrive1`) used by the tray, the
//! flyout, the CLI and the Nautilus extension. Rich results are JSON.

use std::fmt::Display;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::{broadcast::error::RecvError, watch};
use zbus::{fdo, interface, object_server::SignalEmitter};

use crate::config::{Config, DBUS_NAME, DBUS_PATH};
use crate::engine::Engine;

pub struct SyncService {
    engine: Arc<Engine>,
    quit: watch::Sender<bool>,
}

fn failed(e: impl Display) -> fdo::Error {
    fdo::Error::Failed(format!("{e:#}"))
}

impl SyncService {
    fn ino_for(&self, path: &str) -> fdo::Result<u64> {
        let mount = self.engine.cfg().mount_path();
        let rel = Path::new(path).strip_prefix(&mount).map_err(|_| {
            fdo::Error::InvalidArgs(format!("{path} isn't in your OneDrive folder"))
        })?;
        let rel = rel.to_string_lossy();
        self.engine
            .db
            .with(|s| s.resolve(&rel))
            .map_err(failed)?
            .map(|i| i.ino)
            .ok_or_else(|| fdo::Error::FileNotFound(path.to_string()))
    }
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
    async fn pin(&self, paths: Vec<String>) -> fdo::Result<()> {
        for p in paths {
            let ino = self.ino_for(&p)?;
            self.engine.set_pinned(ino, true).map_err(failed)?;
        }
        Ok(())
    }

    /// Untick "Always keep on this device" (the files stay local for now).
    async fn unpin(&self, paths: Vec<String>) -> fdo::Result<()> {
        for p in paths {
            let ino = self.ino_for(&p)?;
            self.engine.set_pinned(ino, false).map_err(failed)?;
        }
        Ok(())
    }

    /// "Free up space". Returns how many files had to stay local.
    async fn free_up_space(&self, paths: Vec<String>) -> fdo::Result<u32> {
        let mut kept = 0;
        for p in paths {
            let ino = self.ino_for(&p)?;
            kept += self.engine.free_up_space(ino).map_err(failed)?;
        }
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

    /// Re-read config.toml and the sign-in tokens.
    async fn reload(&self) -> fdo::Result<()> {
        let cfg = Config::load(&self.engine.paths.config_file).map_err(failed)?;
        self.engine.set_config(cfg);
        self.engine.reload_tokens().await;
        Ok(())
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
}

/// Claim the bus name and forward engine changes as `Changed` signals.
pub async fn serve(engine: Arc<Engine>, quit: watch::Sender<bool>) -> Result<zbus::Connection> {
    let mut changes = engine.subscribe();
    let conn = zbus::connection::Builder::session()?
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
    fn restore_version(&self, path: String, version_id: String) -> zbus::Result<()>;
    fn pause(&self, seconds: u32) -> zbus::Result<()>;
    fn resume(&self) -> zbus::Result<()>;
    fn sync_now(&self) -> zbus::Result<()>;
    fn reload(&self) -> zbus::Result<()>;
    fn sign_out(&self) -> zbus::Result<()>;
    fn local_usage(&self) -> zbus::Result<u64>;
    fn quit(&self) -> zbus::Result<()>;
    #[zbus(signal)]
    fn changed(&self) -> zbus::Result<()>;
}
