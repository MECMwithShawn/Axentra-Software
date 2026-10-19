//! Turns daemon state into what the flyout and settings show, using the
//! Windows client's wording.

use std::path::Path;

use axentra_onedrive_core::config::Config;
use axentra_onedrive_core::db::Activity;
use axentra_onedrive_core::status::{human_size, Account, Issue, Status};
use slint::{Color, ModelRc, SharedString, VecModel};

use crate::{ActivityRow, Flyout, SettingsWindow};

pub fn relative_time(ts: i64, now: i64) -> String {
    let d = (now - ts).max(0);
    match d {
        0..=59 => "Just now".into(),
        60..=119 => "1 minute ago".into(),
        120..=3599 => format!("{} minutes ago", d / 60),
        3600..=7199 => "1 hour ago".into(),
        7200..=86399 => format!("{} hours ago", d / 3600),
        86400..=172799 => "Yesterday".into(),
        _ => chrono::DateTime::from_timestamp(ts, 0)
            .map(|t| t.with_timezone(&chrono::Local).format("%b %-d").to_string())
            .unwrap_or_default(),
    }
}

/// File-type label and colour for the activity tile.
pub fn tile(name: &str) -> (String, Color) {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let rgb = |r, g, b| Color::from_rgb_u8(r, g, b);
    let color = match ext.as_str() {
        "doc" | "docx" | "odt" | "rtf" => rgb(0x18, 0x5a, 0xbd),
        "xls" | "xlsx" | "ods" | "csv" => rgb(0x10, 0x7c, 0x41),
        "ppt" | "pptx" | "odp" | "key" => rgb(0xc4, 0x3e, 0x1c),
        "pdf" => rgb(0xd1, 0x34, 0x38),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "bmp" | "svg" | "tif" | "tiff"
        | "raw" => rgb(0x87, 0x64, 0xb8),
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" => rgb(0xc2, 0x39, 0xb3),
        "mp3" | "flac" | "wav" | "m4a" | "ogg" | "opus" => rgb(0xe3, 0x00, 0x8c),
        "zip" | "7z" | "tar" | "gz" | "xz" | "rar" | "bz2" | "zst" => rgb(0x8e, 0x56, 0x2e),
        "rs" | "py" | "js" | "ts" | "c" | "h" | "cpp" | "go" | "java" | "sh" | "json" | "toml"
        | "yaml" | "yml" | "html" | "css" => rgb(0x03, 0x83, 0x87),
        _ => rgb(0x61, 0x61, 0x61),
    };
    let mut label = if ext.is_empty() {
        "FILE".to_string()
    } else {
        ext.to_ascii_uppercase()
    };
    label.truncate(4);
    (label, color)
}

fn subtitle(a: &Activity) -> String {
    match a.kind.as_str() {
        "uploaded" => format!("Uploaded to {}", a.folder),
        "downloaded" => "Downloaded to this device".into(),
        "deleted" => format!("Deleted from {}", a.folder),
        "deleted_online" => format!("Removed from {} online", a.folder),
        "added_online" => format!("Added to {}", a.folder),
        "updated_online" => format!("Updated in {}", a.folder),
        "renamed" => format!("Renamed in {}", a.folder),
        "moved" => format!("Moved to {}", a.folder),
        "conflict" => a.detail.clone(),
        "error" => format!("Couldn't sync: {}", a.detail),
        other => other.to_string(),
    }
}

pub fn rows(activity: &[Activity], issues: &[Issue], mount: &Path, now: i64) -> Vec<ActivityRow> {
    let mut out = Vec::new();
    for i in issues {
        let name = i.path.rsplit('/').next().unwrap_or(&i.path).to_string();
        out.push(ActivityRow {
            name: name.into(),
            subtitle: i.message.clone().into(),
            tile: SharedString::default(),
            tile_color: Color::default(),
            folder: false,
            path: mount
                .join(i.path.trim_start_matches('/'))
                .display()
                .to_string()
                .into(),
            gone: false,
            problem: true,
        });
    }
    // Collapse repeats (e.g. downloaded then uploaded) to the latest entry per file.
    let mut seen = std::collections::HashSet::new();
    for a in activity {
        if a.kind == "error" || !seen.insert(a.path.clone()) {
            continue;
        }
        let (label, color) = tile(&a.name);
        let gone = matches!(a.kind.as_str(), "deleted" | "deleted_online");
        out.push(ActivityRow {
            name: a.name.clone().into(),
            subtitle: format!("{} · {}", subtitle(a), relative_time(a.ts, now)).into(),
            tile: label.into(),
            tile_color: color,
            folder: a.is_dir,
            path: mount
                .join(a.path.trim_start_matches('/'))
                .display()
                .to_string()
                .into(),
            gone,
            problem: false,
        });
        if out.len() >= 40 {
            break;
        }
    }
    out
}

pub fn storage_text(a: &Account) -> String {
    if !a.signed_in || a.total == 0 {
        return "Storage information isn't available".into();
    }
    let pct = (a.used as f64 / a.total as f64 * 100.0).round();
    format!(
        "{} used of {} ({}%)",
        human_size(a.used),
        human_size(a.total),
        pct
    )
}

pub fn storage_fraction(a: &Account) -> f32 {
    if a.total == 0 {
        0.0
    } else {
        (a.used as f64 / a.total as f64) as f32
    }
}

pub fn account_label(a: &Account) -> String {
    if !a.signed_in {
        return "Not signed in".into();
    }
    let kind = if a.drive_type == "business" {
        "Work or school"
    } else {
        "Personal"
    };
    if a.name.is_empty() {
        kind.into()
    } else {
        format!("{kind} · {}", a.name)
    }
}

pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

pub fn apply_flyout(ui: &Flyout, st: &Status, acct: &Account, activity: &[Activity]) {
    let now = chrono::Utc::now().timestamp();
    ui.set_state(st.state.clone().into());
    ui.set_headline(st.message.clone().into());
    ui.set_detail(st.detail.clone().into());
    ui.set_progress(if st.bytes_total > 0 {
        (st.bytes_done as f64 / st.bytes_total as f64) as f32
    } else {
        0.0
    });
    ui.set_account_label(account_label(acct).into());
    ui.set_storage_text(storage_text(acct).into());
    ui.set_storage_fraction(storage_fraction(acct));
    let rows = rows(activity, &st.issues, Path::new(&st.mount_point), now);
    ui.set_rows(ModelRc::new(VecModel::from(rows)));
}

pub const POLL_CHOICES: [u64; 4] = [10, 30, 60, 300];

pub fn apply_settings(
    ui: &SettingsWindow,
    cfg: &Config,
    st: &Status,
    acct: &Account,
    local_usage: u64,
) {
    ui.set_paused(st.state == "paused");
    ui.set_files_on_demand(cfg.files_on_demand);
    ui.set_folder_path(cfg.mount_path().display().to_string().into());
    ui.set_local_usage(human_size(local_usage).into());
    ui.set_signed_in(acct.signed_in);
    ui.set_account_name(acct.name.clone().into());
    ui.set_account_email(acct.email.clone().into());
    ui.set_account_type(acct.title().into());
    ui.set_account_initials(initials(&acct.name).into());
    ui.set_storage_text(storage_text(acct).into());
    ui.set_storage_fraction(storage_fraction(acct));
    ui.set_notify_deletes(cfg.notify_deletes);
    ui.set_notify_conflicts(cfg.notify_conflicts);
    ui.set_notify_errors(cfg.notify_errors);
    ui.set_upload_mode((cfg.upload_limit_kbps > 0) as i32);
    if cfg.upload_limit_kbps > 0 {
        ui.set_upload_kbps(cfg.upload_limit_kbps as i32);
    }
    ui.set_download_mode((cfg.download_limit_kbps > 0) as i32);
    if cfg.download_limit_kbps > 0 {
        ui.set_download_kbps(cfg.download_limit_kbps as i32);
    }
    let poll = POLL_CHOICES
        .iter()
        .position(|p| *p >= cfg.poll_interval_secs)
        .unwrap_or(POLL_CHOICES.len() - 1);
    ui.set_poll_index(poll as i32);
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    ui.set_notice(
        "Axentra OneDrive is an independent, open-source OneDrive client for Linux. It talks to OneDrive \
         through the documented Microsoft Graph API and isn't made, endorsed or supported by \
         Microsoft. OneDrive is a trademark of Microsoft Corporation."
            .into(),
    );
}

/// Read the settings window back into a config.
pub fn read_settings(ui: &SettingsWindow, cfg: &mut Config) {
    cfg.notify_deletes = ui.get_notify_deletes();
    cfg.notify_conflicts = ui.get_notify_conflicts();
    cfg.notify_errors = ui.get_notify_errors();
    cfg.upload_limit_kbps = if ui.get_upload_mode() == 1 {
        ui.get_upload_kbps().max(1) as u64
    } else {
        0
    };
    cfg.download_limit_kbps = if ui.get_download_mode() == 1 {
        ui.get_download_kbps().max(1) as u64
    } else {
        0
    };
    let i = ui.get_poll_index().clamp(0, POLL_CHOICES.len() as i32 - 1) as usize;
    cfg.poll_interval_secs = POLL_CHOICES[i];
    cfg.files_on_demand = ui.get_files_on_demand();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(relative_time(100, 130), "Just now");
        assert_eq!(relative_time(0, 150), "2 minutes ago");
        assert_eq!(relative_time(0, 3600 * 5), "5 hours ago");
        assert_eq!(relative_time(0, 90_000), "Yesterday");
    }

    #[test]
    fn tiles_and_initials() {
        assert_eq!(tile("Budget.XLSX").0, "XLSX");
        assert_eq!(tile("archive.tar.gz").0, "GZ");
        assert_eq!(tile("Makefile").0, "FILE");
        assert_eq!(initials("Test User"), "TU");
        assert_eq!(initials("cher"), "C");
    }
}
