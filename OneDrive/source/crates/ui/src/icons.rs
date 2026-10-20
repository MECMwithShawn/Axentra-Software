//! Puts the chosen app icon colour into the user's icon theme, so the app
//! menu, notifications and file managers show it too, and into the app's
//! card in Axentra Command.

use std::path::PathBuf;
use std::process::Command;

const SIZES: [u32; 8] = [16, 24, 32, 48, 64, 128, 256, 512];

fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
}

/// Give the Axentra Command card (installed by install.sh) the colour's accent.
fn tint_command_card(color: &str) -> std::io::Result<()> {
    let path = data_dir().join("axentra/command/apps/onedrive.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let Ok(mut card) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(());
    };
    let [r, g, b] = axentra_onedrive_core::config::icon_accent(color);
    let accent = serde_json::Value::from(format!("#{r:02X}{g:02X}{b:02X}"));
    if card.get("accent") != Some(&accent) {
        card["accent"] = accent;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&card)? + "\n")?;
        std::fs::rename(tmp, path)?;
    }
    Ok(())
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
    tint_command_card(color)?;
    let theme = data_dir().join("icons/hicolor");
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
