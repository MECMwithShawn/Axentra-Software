//! Pin to desktop (Settings › General › Window): the window stays below the
//! others. GNOME on Wayland ignores that request from Wayland windows, so a
//! pinned window opens through XWayland instead, where GNOME honours it. The
//! choice is remembered in a marker file next to the database
//! (`<db>.pinned`), read at the very start of `main` before GTK starts.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The marker that says this profile is pinned.
pub fn marker(db: &Path) -> PathBuf {
    let mut p = db.as_os_str().to_owned();
    p.push(".pinned");
    PathBuf::from(p)
}

/// Remember the setting for the next start. Errors are only logged: the
/// window state was already changed for this run.
pub fn remember(db: &Path, pinned: bool) {
    let m = marker(db);
    let r = if pinned { std::fs::write(&m, b"") } else { std::fs::remove_file(&m).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) };
    if let Err(e) = r {
        tracing::warn!("pin to desktop: {}: {e}", m.display());
    }
}

/// Whether this start should use XWayland: the profile is pinned, the
/// session is Wayland, and nobody chose a GDK backend already.
pub fn wants_xwayland(db: &Path, env: impl Fn(&str) -> Option<OsString>) -> bool {
    env("GDK_BACKEND").is_none() && on_wayland(&env) && env("DISPLAY").is_some() && marker(db).exists()
}

fn on_wayland(env: &impl Fn(&str) -> Option<OsString>) -> bool {
    env("WAYLAND_DISPLAY").is_some() || env("XDG_SESSION_TYPE").is_some_and(|t| t == "wayland")
}

/// Whether changing the pin takes effect only after a restart: a Wayland
/// window can't be kept below, so the next start goes through XWayland.
pub fn needs_restart(env: impl Fn(&str) -> Option<OsString>) -> bool {
    on_wayland(&env) && env("GDK_BACKEND").is_none_or(|b| b != "x11")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let m: HashMap<String, OsString> = vars.iter().map(|(k, v)| (k.to_string(), OsString::from(v))).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn pinned_profiles_open_through_xwayland_only_on_wayland() {
        let dir = std::env::temp_dir().join(format!("axentra-pin-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("a.db");
        let wayland = [("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")];
        assert!(!wants_xwayland(&db, env(&wayland)));
        remember(&db, true);
        assert!(marker(&db).exists());
        assert!(wants_xwayland(&db, env(&wayland)));
        assert!(!wants_xwayland(&db, env(&[("DISPLAY", ":0")])), "X11 session: nothing to change");
        assert!(!wants_xwayland(&db, env(&[("WAYLAND_DISPLAY", "wayland-0")])), "no XWayland");
        assert!(!wants_xwayland(&db, env(&[("WAYLAND_DISPLAY", "w"), ("DISPLAY", ":0"), ("GDK_BACKEND", "wayland")])), "backend chosen by the user");
        remember(&db, false);
        remember(&db, false);
        assert!(!marker(&db).exists());
        assert!(needs_restart(env(&wayland)));
        assert!(!needs_restart(env(&[("WAYLAND_DISPLAY", "w"), ("GDK_BACKEND", "x11")])));
        assert!(!needs_restart(env(&[("DISPLAY", ":0")])));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
