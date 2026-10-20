//! Turns daemon state into what the flyout and settings show, using the
//! Windows client's wording.

use std::path::Path;

use axentra_onedrive_core::config::Config;
use axentra_onedrive_core::db::Activity;
use axentra_onedrive_core::status::{human_size, Account, Issue, Status};
use slint::{ModelRc, SharedString, VecModel};

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

/// Which file-type icon a name gets in the activity list.
pub fn file_kind(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "bmp" | "svg" | "tif" | "tiff"
        | "raw" | "avif" | "ico" => "image",
        "doc" | "docx" | "odt" | "rtf" => "doc",
        "xls" | "xlsx" | "ods" | "csv" => "sheet",
        "ppt" | "pptx" | "odp" | "key" => "slides",
        "pdf" => "pdf",
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" => "video",
        "mp3" | "flac" | "wav" | "m4a" | "ogg" | "opus" => "audio",
        "zip" | "7z" | "tar" | "gz" | "xz" | "rar" | "bz2" | "zst" => "archive",
        "rs" | "py" | "js" | "ts" | "c" | "h" | "cpp" | "go" | "java" | "sh" | "json" | "toml"
        | "yaml" | "yml" | "html" | "css" | "xml" => "code",
        "txt" | "md" | "log" => "text",
        _ => "generic",
    }
}

/// What happened, and the folder it's linked to ("Uploaded to", "Documents").
fn action(a: &Activity) -> (String, String) {
    let verb = match a.kind.as_str() {
        "uploaded" => "Uploaded to",
        "downloaded" => "Downloaded to",
        "deleted" => "Deleted from",
        "deleted_online" => "Deleted online from",
        "added_online" => "Added to",
        "updated_online" => "Updated in",
        "renamed" => "Renamed in",
        "moved" => "Moved to",
        "conflict" => return (a.detail.clone(), String::new()),
        other => return (other.to_string(), String::new()),
    };
    (verb.into(), a.folder.clone())
}

/// The folder holding `rel` (a path inside OneDrive), on this device.
fn parent_dir(mount: &Path, rel: &str) -> String {
    let rel = rel.trim_start_matches('/');
    match rel.rsplit_once('/') {
        Some((dir, _)) => mount.join(dir).display().to_string(),
        None => mount.display().to_string(),
    }
}

pub fn rows(activity: &[Activity], issues: &[Issue], mount: &Path, now: i64) -> Vec<ActivityRow> {
    let mut out = Vec::new();
    for i in issues {
        let name = i.path.rsplit('/').next().unwrap_or(&i.path).to_string();
        out.push(ActivityRow {
            kind: file_kind(&name).into(),
            name: name.into(),
            action: i.message.clone().into(),
            folder: SharedString::default(),
            folder_path: parent_dir(mount, &i.path).into(),
            time: SharedString::default(),
            is_folder: false,
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
        let (verb, folder) = action(a);
        out.push(ActivityRow {
            name: a.name.clone().into(),
            action: verb.into(),
            folder: folder.into(),
            folder_path: parent_dir(mount, &a.path).into(),
            time: relative_time(a.ts, now).into(),
            kind: file_kind(&a.name).into(),
            is_folder: a.is_dir,
            path: mount
                .join(a.path.trim_start_matches('/'))
                .display()
                .to_string()
                .into(),
            gone: matches!(a.kind.as_str(), "deleted" | "deleted_online"),
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
        "{} of {} used ({}%)",
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
    ui.set_account_title(if acct.signed_in {
        acct.title().into()
    } else {
        "OneDrive".into()
    });
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
    fn kinds_and_initials() {
        assert_eq!(file_kind("Budget.XLSX"), "sheet");
        assert_eq!(file_kind("archive.tar.gz"), "archive");
        assert_eq!(file_kind("Makefile"), "generic");
        assert_eq!(file_kind("Beach.png"), "image");
        assert_eq!(initials("Test User"), "TU");
        assert_eq!(initials("cher"), "C");
    }

    #[test]
    fn rows_link_the_folder() {
        let a = Activity {
            seq: 1,
            ts: 0,
            kind: "downloaded".into(),
            name: "Beach.png".into(),
            folder: "Trip photos".into(),
            path: "/Trip photos/Beach.png".into(),
            is_dir: false,
            detail: String::new(),
        };
        let top = Activity {
            path: "/notes.txt".into(),
            name: "notes.txt".into(),
            folder: "OneDrive".into(),
            kind: "uploaded".into(),
            ..a.clone()
        };
        let r = rows(&[a, top], &[], Path::new("/home/u/OneDrive"), 7200);
        assert_eq!(r[0].action, "Downloaded to");
        assert_eq!(r[0].folder, "Trip photos");
        assert_eq!(r[0].folder_path, "/home/u/OneDrive/Trip photos");
        assert_eq!(r[0].time, "2 hours ago");
        assert_eq!(r[0].kind, "image");
        assert_eq!(r[1].action, "Uploaded to");
        assert_eq!(r[1].folder_path, "/home/u/OneDrive");
    }
}
