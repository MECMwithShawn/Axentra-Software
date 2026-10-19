//! The background service: mount, D-Bus API and sync loop, until signalled.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::watch;

use crate::config::{Config, Paths};
use crate::engine::{Engine, Options};

pub struct DaemonOptions {
    pub dbus: bool,
    pub notifications: bool,
}

impl Default for DaemonOptions {
    fn default() -> Self {
        Self {
            dbus: true,
            notifications: true,
        }
    }
}

pub async fn run(paths: Paths, opts: DaemonOptions) -> Result<()> {
    let cfg = Config::load(&paths.config_file)?;
    if cfg.client_id.is_empty() {
        tracing::warn!(
            "no client_id in {}; run `axentra-onedrive setup` to connect an app registration",
            paths.config_file.display()
        );
    }
    let engine = Engine::new(
        paths,
        cfg,
        Options {
            notifications: opts.notifications,
        },
    )?;
    let (quit_tx, mut quit_rx) = watch::channel(false);

    let _bus = if opts.dbus {
        Some(
            crate::ipc::serve(engine.clone(), quit_tx.clone())
                .await
                .context(
                    "couldn't register on the session bus (is Axentra OneDrive already running?)",
                )?,
        )
    } else {
        None
    };

    let mount = engine.cfg().mount_path();
    let session = {
        let e = engine.clone();
        let m = mount.clone();
        tokio::task::spawn_blocking(move || crate::fs::mount(e, &m)).await??
    };
    engine.attach_kernel(session.notifier());
    tracing::info!("your OneDrive is at {}", mount.display());

    let sync = tokio::spawn(engine.clone().run(quit_rx.clone()));

    let mut term = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
        _ = quit_rx.wait_for(|q| *q) => {}
    }
    tracing::info!("shutting down");
    let _ = quit_tx.send(true);
    if tokio::time::timeout(std::time::Duration::from_secs(10), sync)
        .await
        .is_err()
    {
        tracing::warn!("sync didn't stop in time; unmounting anyway");
    }
    unmount(session, &mount).await;
    Ok(())
}

async fn unmount(session: fuser::BackgroundSession, mount: &Path) {
    let m = mount.to_path_buf();
    let clean = tokio::task::spawn_blocking(move || session.umount_and_join()).await;
    if !matches!(clean, Ok(Ok(()))) {
        // Busy (a shell is inside it, a file is open): detach lazily.
        let _ = std::process::Command::new("fusermount3")
            .arg("-uz")
            .arg(&m)
            .status();
    }
}

/// For tests and the demo: an engine and mount without the bus.
pub fn start_mounted(engine: Arc<Engine>) -> Result<fuser::BackgroundSession> {
    let mount = engine.cfg().mount_path();
    let session = crate::fs::mount(engine.clone(), &mount)?;
    engine.attach_kernel(session.notifier());
    Ok(session)
}
