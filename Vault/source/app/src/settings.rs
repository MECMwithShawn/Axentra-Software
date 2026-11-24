//! Non-secret preferences in %APPDATA%\Axentra Vault: the last opened database path, the
//! interface layout and where backups go. Nothing about vault contents is ever stored here.

use std::path::PathBuf;

use vault_core::backups::{self, BackupConfig};

/// `%APPDATA%`. Tests get a throwaway folder instead, so they never touch the real one.
fn app_data() -> Option<PathBuf> {
    #[cfg(test)]
    crate::ui_tests::isolate_settings();
    #[cfg(all(target_os="linux",not(test)))]
    return Some(crate::linux::config_home());
    #[cfg(any(windows,test))]
    std::env::var_os("APPDATA").map(PathBuf::from)
}

/// `%LOCALAPPDATA%` (not synced), which holds the default backup root; isolated like
/// [`app_data`] in tests.
fn local_app_data() -> Option<PathBuf> {
    #[cfg(test)]
    crate::ui_tests::isolate_settings();
    #[cfg(all(target_os="linux",not(test)))]
    return Some(crate::linux::data_home());
    #[cfg(any(windows,test))]
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Local"))
        })
}

pub(crate) fn dir() -> Option<PathBuf> {
    app_data().map(|d| d.join(if cfg!(windows) { "Axentra Vault" } else { "vault" }))
}

/// Where backups go: the default root `%LOCALAPPDATA%\Axentra Vault\Backups`, plus the chosen
/// and earlier roots and the folder of each database from `backups.json` beside these settings.
pub fn backup_config() -> BackupConfig {
    let default_root =
        backups::default_root_in(&local_app_data().unwrap_or_else(std::env::temp_dir));
    match dir() {
        Some(d) => BackupConfig::load(default_root, d.join(backups::SETTINGS_FILE)),
        None => BackupConfig::new(default_root),
    }
}

fn read(name: &str) -> Option<String> {
    let s = std::fs::read_to_string(dir()?.join(name)).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

fn write(name: &str, value: &str) {
    if let Some(d) = dir() {
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::write(d.join(name), value);
    }
}

pub fn last_database() -> Option<String> {
    read("last-database.txt")
}

pub fn remember_database(path: &str) {
    write("last-database.txt", path);
}

/// Interface layout: the Floating Modular design, or the original Classic layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Floating,
    Classic,
}

pub fn layout() -> Layout {
    match read("layout.txt").as_deref() {
        Some("classic") => Layout::Classic,
        _ => Layout::Floating,
    }
}

pub fn set_layout(layout: Layout) {
    write(
        "layout.txt",
        if layout == Layout::Classic {
            "classic"
        } else {
            "floating"
        },
    );
}

/// Columns shown in the Classic layout's entry table (names only).
pub fn columns() -> Vec<crate::columns::Col> {
    match read("columns.txt") {
        Some(text) => crate::columns::from_text(&text),
        None => crate::columns::defaults(),
    }
}

pub fn set_columns(cols: &[crate::columns::Col]) {
    write("columns.txt", &crate::columns::to_text(cols));
}

/// Hide the app's windows from screenshots, screen recording and screen sharing (default on).
pub fn hide_from_capture() -> bool {
    crate::hardening::capture_supported() && read("capture.txt").as_deref() != Some("show")
}

pub fn set_hide_from_capture(hide: bool) {
    write("capture.txt", if hide { "hide" } else { "show" });
}

/// Classic table sort: the column (None = Title) and whether it is descending.
pub fn table_sort() -> (Option<crate::columns::Col>, bool) {
    let Some(text) = read("sort.txt") else {
        return (None, false);
    };
    // "<column key> asc|desc"; keys of custom fields may contain spaces.
    let (key, dir) = text.rsplit_once(' ').unwrap_or((text.as_str(), "asc"));
    let col = if key == "title" {
        None
    } else {
        match crate::columns::Col::parse(key) {
            Some(c) => Some(c),
            None => return (None, false),
        }
    };
    (col, dir == "desc")
}

pub fn set_table_sort(col: Option<&crate::columns::Col>, desc: bool) {
    let key = col.map_or_else(|| "title".to_owned(), crate::columns::Col::key);
    write(
        "sort.txt",
        &format!("{key} {}", if desc { "desc" } else { "asc" }),
    );
}

/// How long the last unlock of `db` took, used to show real progress next time.
pub fn unlock_ms(db: &str) -> Option<u64> {
    read("unlock-times.txt")?
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .find(|(_, path)| path.eq_ignore_ascii_case(db))
        .and_then(|(ms, _)| ms.parse().ok())
}

pub fn set_unlock_ms(db: &str, ms: u64) {
    let mut lines: Vec<String> = read("unlock-times.txt")
        .unwrap_or_default()
        .lines()
        .filter(|l| {
            l.split_once('\t')
                .is_some_and(|(_, p)| !p.eq_ignore_ascii_case(db))
        })
        .map(str::to_owned)
        .collect();
    lines.insert(0, format!("{ms}\t{db}"));
    lines.truncate(20);
    write("unlock-times.txt", &lines.join("\n"));
}
