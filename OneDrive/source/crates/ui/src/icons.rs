//! Puts the chosen app icon colour into the user's icon theme, so the app
//! menu, notifications and file managers show it too.

use std::path::PathBuf;
use std::process::Command;

const SIZES: [u32; 8] = [16, 24, 32, 48, 64, 128, 256, 512];

fn theme_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("icons/hicolor")
}

/// Install `color`'s icons as `axentra-onedrive`. Returns whether anything
/// changed; does nothing when running from a build tree without icon sets.
pub fn apply(color: &str) -> std::io::Result<bool> {
    let Some(app_dir) = std::env::current_exe()?.parent().map(|d| d.to_path_buf()) else {
        return Ok(false);
    };
    let source = app_dir.join("icons").join(color);
    if !source.is_dir() {
        return Ok(false);
    }
    let theme = theme_dir();
    let mut changed = false;
    for n in SIZES {
        let icon = std::fs::read(source.join(format!("axentra-onedrive-{n}.png")))?;
        let target = theme.join(format!("{n}x{n}/apps/axentra-onedrive.png"));
        if std::fs::read(&target).ok().as_deref() != Some(icon.as_slice()) {
            std::fs::create_dir_all(target.parent().expect("icon path has a parent"))?;
            std::fs::write(&target, icon)?;
            changed = true;
        }
    }
    if changed {
        // Refreshing the cache also tells running desktops to reload icons.
        let _ = Command::new("gtk-update-icon-cache")
            .args(["-q", "-t", "-f"])
            .arg(&theme)
            .status();
    }
    Ok(changed)
}
