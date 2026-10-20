//! The background service: mount, D-Bus API and sync loop, until signalled.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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
    if tokio::time::timeout(Duration::from_secs(10), sync)
        .await
        .is_err()
    {
        tracing::warn!("sync didn't stop in time; unmounting anyway");
    }
    unmount(session, &mount, Duration::from_secs(5)).await;
    Ok(())
}

/// Unmount, waiting at most `wait` for whatever still has files or folders
/// open inside (a folder window, a search, a shell). fuser detaches the
/// mount lazily and then serves those until they close, so waiting for them
/// could keep the service from stopping at all. Once this process exits they
/// get an error instead, like files on a disconnected drive.
pub async fn unmount(session: fuser::BackgroundSession, mount: &Path, wait: Duration) {
    let users = holders(mount);
    let (tx, rx) = tokio::sync::oneshot::channel();
    // A plain thread: the runtime waits for spawn_blocking tasks on exit.
    std::thread::spawn(move || {
        let _ = tx.send(session.umount_and_join());
    });
    match tokio::time::timeout(wait, rx).await {
        Ok(Ok(Ok(()))) => return,
        Ok(_) => {}
        Err(_) => tracing::warn!(
            "{} was still in use{}; closing it anyway",
            mount.display(),
            if users.is_empty() {
                String::new()
            } else {
                format!(" by {}", users.join(", "))
            }
        ),
    }
    if is_mounted(mount) {
        let _ = std::process::Command::new("fusermount3")
            .arg("-uz")
            .arg(mount)
            .status();
    }
}

/// Processes with a file, folder or working directory inside `mount`.
fn holders(mount: &Path) -> Vec<String> {
    let inside = |link: PathBuf| link.starts_with(mount);
    let mut found = Vec::new();
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return found;
    };
    for p in procs.flatten() {
        let Some(pid) = p.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let dir = p.path();
        let busy = std::fs::read_link(dir.join("cwd")).is_ok_and(inside)
            || std::fs::read_dir(dir.join("fd")).is_ok_and(|fds| {
                fds.flatten()
                    .any(|fd| std::fs::read_link(fd.path()).is_ok_and(inside))
            });
        if busy && pid != std::process::id() {
            found.push(crate::thumbs::describe(pid));
        }
    }
    found
}

fn is_mounted(mount: &Path) -> bool {
    // Field 5 of mountinfo, with space, tab, newline and backslash octal-escaped.
    let mut want = String::new();
    for c in mount.to_string_lossy().chars() {
        match c {
            ' ' | '\t' | '\n' | '\\' => want.push_str(&format!("\\{:03o}", c as u32)),
            c => want.push(c),
        }
    }
    std::fs::read_to_string("/proc/self/mountinfo").is_ok_and(|t| {
        t.lines()
            .any(|l| l.split(' ').nth(4) == Some(want.as_str()))
    })
}

/// For tests and the demo: an engine and mount without the bus.
pub fn start_mounted(engine: Arc<Engine>) -> Result<fuser::BackgroundSession> {
    let mount = engine.cfg().mount_path();
    let session = crate::fs::mount(engine.clone(), &mount)?;
    engine.attach_kernel(session.notifier());
    Ok(session)
}
