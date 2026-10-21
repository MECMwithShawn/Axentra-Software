//! Turns daemon state into what the flyout and settings show, using the
//! Windows client's wording.

use std::path::Path;

use axentra_onedrive_core::config::Config;
use axentra_onedrive_core::db::Activity;
use axentra_onedrive_core::status::{human_size, Account, Issue, Status};
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::{ActivityRow, Flyout, SettingsWindow};

/// The most rows the panel lists, sync problems included.
const MAX_ROWS: usize = 40;

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
    for i in issues.iter().take(MAX_ROWS) {
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
        if out.len() >= MAX_ROWS {
            break;
        }
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
    }
    out
}

/// The panel's list, as `update_rows` changes it. Slint keeps a row's item
/// (and the menu open on it) through `set`, and makes a new one on `insert`.
trait RowList {
    fn get(&self, i: usize) -> Option<ActivityRow>;
    fn set(&self, i: usize, row: ActivityRow);
    fn insert(&self, i: usize, row: ActivityRow);
    fn remove(&self, i: usize);
}

impl RowList for VecModel<ActivityRow> {
    fn get(&self, i: usize) -> Option<ActivityRow> {
        self.row_data(i)
    }
    fn set(&self, i: usize, row: ActivityRow) {
        self.set_row_data(i, row);
    }
    fn insert(&self, i: usize, row: ActivityRow) {
        VecModel::insert(self, i, row);
    }
    fn remove(&self, i: usize) {
        VecModel::remove(self, i);
    }
}

/// Bring the list shown in the panel up to date without replacing it:
/// for a new model Slint rebuilds every row, which closes a row's open
/// menu and drops a click in progress. An item only ever shows one file,
/// so its menu can't act on another: a file that moved up the list is
/// removed and inserted again, and only rows whose text changed are rewritten.
fn update_rows(list: &impl RowList, rows: &[ActivityRow]) {
    let same = |a: &ActivityRow, b: &ActivityRow| a.path == b.path && a.problem == b.problem;
    let mut i = 0;
    while let Some(new) = rows.get(i) {
        match list.get(i) {
            Some(old) if same(&old, new) => {
                if old != *new {
                    list.set(i, new.clone());
                }
            }
            // Still listed further down, so `new` goes in above it.
            Some(old) if rows[i + 1..].iter().any(|r| same(r, &old)) => list.insert(i, new.clone()),
            // Gone, or moved up: compare the next one.
            Some(_) => {
                list.remove(i);
                continue;
            }
            None => list.insert(i, new.clone()),
        }
        i += 1;
    }
    while list.get(rows.len()).is_some() {
        list.remove(rows.len());
    }
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

/// Show the service's state in the panel. The recent files are only
/// updated when `activity` was fetched, which happens while the panel is open.
pub fn apply_flyout(ui: &Flyout, st: &Status, acct: &Account, activity: Option<&[Activity]>) {
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
    let Some(activity) = activity else { return };
    let now = chrono::Utc::now().timestamp();
    let rows = rows(activity, &st.issues, Path::new(&st.mount_point), now);
    let model = ui.get_rows();
    match model.as_any().downcast_ref::<VecModel<ActivityRow>>() {
        Some(model) => update_rows(model, &rows),
        None => ui.set_rows(ModelRc::new(VecModel::from(rows))),
    }
}

pub const POLL_CHOICES: [u64; 4] = [10, 30, 60, 300];

/// About how much more "Download all files" would download, or "" if unknown.
pub fn download_size(acct: &Account, local_usage: u64) -> String {
    if !acct.signed_in || acct.used == 0 {
        return String::new();
    }
    human_size(acct.used.saturating_sub(local_usage))
}

/// The OneDrive folder for Settings, and where it moves to if config.toml
/// has a new location the running service doesn't use until it restarts.
/// `st.mount_point` is the folder the service has, if it's running.
fn folder_text(cfg: &Config, st: &Status) -> (String, String) {
    let wanted = cfg.mount_path();
    if st.mount_point.is_empty() || Path::new(&st.mount_point) == wanted {
        return (wanted.display().to_string(), String::new());
    }
    (
        st.mount_point.clone(),
        format!(
            "Moves to {} when the sync service restarts.",
            wanted.display()
        ),
    )
}

/// What Settings shows that changes while it's open. `local_usage` is
/// `None` when it wasn't fetched.
pub fn apply_live(
    ui: &SettingsWindow,
    cfg: &Config,
    st: &Status,
    acct: &Account,
    local_usage: Option<u64>,
) {
    let (folder, moves) = folder_text(cfg, st);
    ui.set_folder_path(folder.into());
    ui.set_folder_moves(moves.into());
    ui.set_paused(st.state == "paused");
    if let Some(usage) = local_usage {
        ui.set_local_usage(human_size(usage).into());
        ui.set_download_size(download_size(acct, usage).into());
    }
    ui.set_signed_in(acct.signed_in);
    ui.set_account_name(acct.name.clone().into());
    ui.set_account_email(acct.email.clone().into());
    ui.set_account_type(acct.title().into());
    ui.set_account_initials(initials(&acct.name).into());
    ui.set_storage_text(storage_text(acct).into());
    ui.set_storage_fraction(storage_fraction(acct));
}

/// Everything Settings shows, when it opens: the live values and the
/// settings from `cfg`.
pub fn apply_settings(
    ui: &SettingsWindow,
    cfg: &Config,
    st: &Status,
    acct: &Account,
    local_usage: Option<u64>,
) {
    apply_live(ui, cfg, st, acct, local_usage);
    ui.set_files_on_demand(cfg.files_on_demand);
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

pub const NOT_RUNNING: &str = "Axentra OneDrive isn't running";

/// What "Free up space" says about the files it had to keep, if any.
pub fn kept_note(kept: u32) -> Option<String> {
    match kept {
        0 => None,
        1 => Some(
            "1 file is still on this device because it's open or has changes that haven't \
             uploaded yet."
                .into(),
        ),
        n => Some(format!(
            "{n} files are still on this device because they're open or have changes that \
             haven't uploaded yet."
        )),
    }
}

/// What went wrong, in words for the user: the sync service's own message,
/// without the D-Bus error name in front of it.
pub fn error_text(e: &anyhow::Error) -> String {
    use zbus::DBusError;
    match e.downcast_ref::<zbus::Error>() {
        Some(zbus::Error::MethodError(name, detail, _)) => match name.as_str() {
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner" => NOT_RUNNING.into(),
            "org.freedesktop.DBus.Error.FileNotFound" => "That item isn't in your OneDrive".into(),
            _ => detail.clone().unwrap_or_else(|| e.to_string()),
        },
        Some(zbus::Error::FDO(fdo)) => fdo.description().unwrap_or_default().to_string(),
        _ => e.to_string(),
    }
}

/// Why config.toml couldn't be read, for Settings: a hand edit with a
/// mistake in it ("TOML parse error at line 3, ...") or a file that can't
/// be opened.
pub fn config_problem(e: &anyhow::Error) -> String {
    let cause = e.root_cause().to_string();
    // Config::load names the line itself: "<path> has a mistake on line N: <what>".
    if let Some((line, what)) = cause
        .split_once(" has a mistake on line ")
        .and_then(|(_, rest)| rest.split_once(": "))
    {
        return format!(
            "config.toml has a mistake on line {line} ({what}), so changes here can't be \
             saved. Fix the file, then try again."
        );
    }
    let lines: Vec<&str> = cause
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let first = lines.first().copied().unwrap_or_default();
    let line = first
        .strip_prefix("TOML parse error at line ")
        .and_then(|rest| rest.split(',').next());
    match (line, lines.last()) {
        (Some(line), Some(what)) => format!(
            "config.toml has a mistake on line {line} ({what}), so changes here can't be \
             saved. Fix the file, then try again."
        ),
        _ => format!("config.toml couldn't be read ({first}), so changes here can't be saved."),
    }
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

    fn row(name: &str, time: &str) -> ActivityRow {
        ActivityRow {
            name: name.into(),
            path: format!("/home/u/OneDrive/{name}").into(),
            time: time.into(),
            ..Default::default()
        }
    }

    fn names(model: &VecModel<ActivityRow>) -> Vec<String> {
        model.iter().map(|r| r.name.to_string()).collect()
    }

    #[test]
    fn rows_update_in_place() {
        let model = VecModel::from(vec![row("a", "Just now"), row("b", "1 minute ago")]);
        // Nothing changed: nothing to do.
        let before: Vec<ActivityRow> = model.iter().collect();
        update_rows(&model, &before);
        assert_eq!(model.iter().collect::<Vec<_>>(), before);

        // A new file at the top is inserted; the others only get new times.
        update_rows(
            &model,
            &[
                row("new", "Just now"),
                row("a", "1 minute ago"),
                row("b", "2 minutes ago"),
            ],
        );
        assert_eq!(names(&model), ["new", "a", "b"]);
        assert_eq!(model.row_data(1).unwrap().time, "1 minute ago");

        // A problem that went away at the top is removed.
        update_rows(
            &model,
            &[row("a", "1 minute ago"), row("b", "2 minutes ago")],
        );
        assert_eq!(names(&model), ["a", "b"]);

        // Fewer rows: trimmed from the end.
        update_rows(&model, &[row("a", "1 minute ago")]);
        assert_eq!(names(&model), ["a"]);
        update_rows(&model, &[]);
        assert_eq!(model.row_count(), 0);
        update_rows(&model, &[row("c", "Just now")]);
        assert_eq!(names(&model), ["c"]);
    }

    /// Stands in for Slint's list: each row is an item, numbered when it's
    /// made, that keeps its number through `set`.
    #[derive(Default)]
    struct Items(std::cell::RefCell<(Vec<(u32, ActivityRow)>, u32)>);

    impl RowList for Items {
        fn get(&self, i: usize) -> Option<ActivityRow> {
            self.0.borrow().0.get(i).map(|(_, r)| r.clone())
        }
        fn set(&self, i: usize, row: ActivityRow) {
            let item = &mut self.0.borrow_mut().0[i].1;
            assert_eq!(item.path, row.path, "an item was given another file");
            *item = row;
        }
        fn insert(&self, i: usize, row: ActivityRow) {
            let (items, made) = &mut *self.0.borrow_mut();
            *made += 1;
            items.insert(i, (*made, row));
        }
        fn remove(&self, i: usize) {
            self.0.borrow_mut().0.remove(i);
        }
    }

    impl Items {
        /// Each row's item number and file, e.g. "1b 2a".
        fn ids(&self) -> String {
            let items = &self.0.borrow().0;
            let ids: Vec<String> = items
                .iter()
                .map(|(n, r)| format!("{n}{}", r.name))
                .collect();
            ids.join(" ")
        }
    }

    #[test]
    fn a_file_that_moves_up_gets_a_new_item() {
        let items = Items::default();
        let list = |names: &str, time: &str| -> Vec<ActivityRow> {
            names.split(' ').map(|n| row(n, time)).collect()
        };
        update_rows(&items, &list("b a c d", "Just now"));
        assert_eq!(items.ids(), "1b 2a 3c 4d");

        // "a" was synced again: its item goes, so a menu open on it closes
        // rather than act on another file, and the other items keep theirs.
        update_rows(&items, &list("a b c d", "1 minute ago"));
        assert_eq!(items.ids(), "5a 1b 3c 4d");
        assert_eq!(items.get(1).unwrap().time, "1 minute ago");

        // The same from the middle, with "a" gone.
        update_rows(&items, &list("c b d", "2 minutes ago"));
        assert_eq!(items.ids(), "6c 1b 4d");
    }

    #[test]
    fn panel_keeps_its_model() {
        // Slint rebuilds every row (closing an open "..." menu) for a new
        // model, so refreshing must keep the one the panel has.
        crate::shot::headless();
        let ui = Flyout::new().unwrap();
        let st = Status {
            mount_point: "/home/u/OneDrive".into(),
            ..Default::default()
        };
        let a = Activity {
            seq: 1,
            ts: chrono::Utc::now().timestamp(),
            kind: "uploaded".into(),
            name: "notes.txt".into(),
            folder: "OneDrive".into(),
            path: "/notes.txt".into(),
            is_dir: false,
            detail: String::new(),
        };
        apply_flyout(
            &ui,
            &st,
            &Account::default(),
            Some(std::slice::from_ref(&a)),
        );
        let first = ui.get_rows();
        apply_flyout(&ui, &st, &Account::default(), Some(&[a.clone(), a]));
        assert!(ui.get_rows() == first);
        assert_eq!(first.row_count(), 1);
        // Without the activity (panel closed) the rows are left alone.
        apply_flyout(&ui, &st, &Account::default(), None);
        assert_eq!(ui.get_rows().row_count(), 1);
    }

    #[test]
    fn rows_are_capped() {
        let issues: Vec<Issue> = (0..100)
            .map(|i| Issue {
                path: format!("/f{i}.txt"),
                message: "Failed".into(),
            })
            .collect();
        assert_eq!(rows(&[], &issues, Path::new("/m"), 0).len(), MAX_ROWS);
    }

    #[test]
    fn config_mistakes_are_explained() {
        let dir = std::env::temp_dir().join(format!("axentra-ui-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "client_id = \"x\"\npoll_interval_secs = \"60\"\n").unwrap();
        let e = Config::load(&path).unwrap_err();
        let text = config_problem(&e);
        assert!(
            text.starts_with("config.toml has a mistake on line 2 ("),
            "{text}"
        );
        assert!(text.contains("expected u64"), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_lose_the_dbus_prefix() {
        let msg = zbus::message::Message::method_call("/", "Versions")
            .unwrap()
            .build(&())
            .unwrap();
        let failed = |name: &str, detail: &str| -> anyhow::Error {
            zbus::Error::MethodError(
                zbus::names::OwnedErrorName::try_from(name).unwrap(),
                Some(detail.into()),
                msg.clone(),
            )
            .into()
        };
        assert_eq!(
            error_text(&failed(
                "org.freedesktop.DBus.Error.Failed",
                "This item hasn't finished uploading yet"
            )),
            "This item hasn't finished uploading yet"
        );
        assert_eq!(
            error_text(&failed(
                "org.freedesktop.DBus.Error.ServiceUnknown",
                "The name is not activatable"
            )),
            NOT_RUNNING
        );
        assert_eq!(error_text(&anyhow::anyhow!("plain")), "plain");
    }

    #[test]
    fn download_size_is_what_isnt_local() {
        let acct = Account {
            signed_in: true,
            used: 10 * 1024 * 1024 * 1024,
            ..Default::default()
        };
        assert_eq!(download_size(&acct, 2 * 1024 * 1024 * 1024), "8.0 GB");
        assert_eq!(download_size(&Account::default(), 0), "");
    }

    #[test]
    fn settings_show_the_folder_the_service_has() {
        let cfg = Config {
            mount_point: "/data/New".into(),
            ..Default::default()
        };
        let running = |mount: &str| Status {
            mount_point: mount.into(),
            ..Default::default()
        };
        // Not running, or running there: just the folder.
        assert_eq!(
            folder_text(&cfg, &Status::default()),
            ("/data/New".into(), String::new())
        );
        assert_eq!(
            folder_text(&cfg, &running("/data/New")),
            ("/data/New".into(), String::new())
        );
        // Moved in config.toml: it's still where it was until a restart.
        let (folder, moves) = folder_text(&cfg, &running("/data/Old"));
        assert_eq!(folder, "/data/Old");
        assert_eq!(moves, "Moves to /data/New when the sync service restarts.");
    }

    #[test]
    fn free_up_space_says_what_it_kept() {
        assert_eq!(kept_note(0), None);
        assert!(kept_note(1).unwrap().starts_with("1 file is still"));
        assert!(kept_note(3).unwrap().starts_with("3 files are still"));
    }
}
