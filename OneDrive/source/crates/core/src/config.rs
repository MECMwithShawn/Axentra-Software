//! User settings (`~/.config/axentra/onedrive/config.toml`) and on-disk locations.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DBUS_NAME: &str = "io.github.mecmwithshawn.AxentraOneDrive";
pub const DBUS_PATH: &str = "/io/github/mecmwithshawn/AxentraOneDrive";
/// App icon colours offered in Settings > Appearance; the first is the default.
pub const ICON_COLORS: [&str; 3] = ["ice", "periwinkle", "azure"];

/// The accent each icon colour is rendered with (also tints the tray cloud).
pub fn icon_accent(color: &str) -> [u8; 3] {
    match color {
        "periwinkle" => [139, 151, 255],
        "azure" => [72, 170, 255],
        _ => [184, 222, 255],
    }
}

pub const DBUS_IFACE: &str = "io.github.mecmwithshawn.AxentraOneDrive1";

/// A problem only the user can fix, such as a settings file that doesn't
/// parse or a OneDrive folder that already has files in it. Restarting the
/// service won't help until they do, so it says why and stays stopped.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct NeedsUser(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Application (client) ID of your Microsoft Entra app registration.
    pub client_id: String,
    /// `common` (personal + work/school), `consumers`, `organizations` or a tenant ID.
    pub tenant: String,
    pub authority: String,
    pub graph_base: String,
    /// Where your OneDrive appears. `~` is expanded.
    pub mount_point: String,
    /// "Save space and download files as you use them". When off, every file
    /// is kept on this device.
    pub files_on_demand: bool,
    /// How often to check OneDrive for changes, in seconds.
    pub poll_interval_secs: u64,
    /// 0 = don't limit.
    pub upload_limit_kbps: u64,
    pub download_limit_kbps: u64,
    pub max_parallel_transfers: usize,
    /// Warn when this many files are deleted online in one go.
    pub delete_warning_threshold: u32,
    pub notify_deletes: bool,
    pub notify_conflicts: bool,
    pub notify_errors: bool,
    /// Glob patterns for files that stay on this device and never upload.
    pub ignore: Vec<String>,
    /// App icon colour: "ice" (default), "periwinkle" or "azure".
    pub icon_color: String,
    /// Show OneDrive's thumbnails of online-only photos and videos in file
    /// managers and file choosers, without downloading the files.
    pub cloud_thumbnails: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            tenant: "common".into(),
            authority: "https://login.microsoftonline.com".into(),
            graph_base: "https://graph.microsoft.com/v1.0".into(),
            mount_point: "~/OneDrive".into(),
            files_on_demand: true,
            poll_interval_secs: 30,
            upload_limit_kbps: 0,
            download_limit_kbps: 0,
            max_parallel_transfers: 4,
            delete_warning_threshold: 50,
            notify_deletes: true,
            notify_conflicts: true,
            notify_errors: true,
            icon_color: "ice".into(),
            cloud_thumbnails: true,
            ignore: vec![
                // Office and LibreOffice lock files, editor swap files, partial downloads.
                "~$*".into(),
                ".~lock.*#".into(),
                ".*.sw?".into(),
                "*.crdownload".into(),
                "*.part".into(),
                ".goutputstream-*".into(),
            ],
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| {
                let line = e
                    .span()
                    .and_then(|s| text.get(..s.start))
                    .map_or(1, |before| before.matches('\n').count() + 1);
                NeedsUser(format!(
                    "{} has a mistake on line {line}: {}",
                    path.display(),
                    e.message()
                ))
                .into()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = format!(
            "# Axentra OneDrive settings. Edit here or in Settings; run `axentra-onedrive reload` after editing by hand.\n{}",
            toml::to_string_pretty(self)?
        );
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    /// The configured icon colour, or the default if it isn't one we ship.
    pub fn icon_color(&self) -> &str {
        if ICON_COLORS.contains(&self.icon_color.as_str()) {
            &self.icon_color
        } else {
            ICON_COLORS[0]
        }
    }

    pub fn mount_path(&self) -> PathBuf {
        normalize(&expand_home(&self.mount_point))
    }

    pub fn auth(&self) -> axentra_onedrive_graph::AuthConfig {
        axentra_onedrive_graph::AuthConfig {
            client_id: self.client_id.clone(),
            tenant: self.tenant.clone(),
            authority: self.authority.clone(),
        }
    }
}

/// `~` and relative paths are taken from the home folder, where systemd
/// starts the service.
pub fn expand_home(p: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    match p.strip_prefix("~/") {
        _ if p == "~" => home,
        Some(rest) => home.join(rest),
        None => home.join(p),
    }
}

/// `p` with `.` and `..` worked out from the text alone, as the shell's
/// `cd ..` does (the OneDrive folder has no symlinks to follow).
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir if out.file_name().is_some() => {
                out.pop();
            }
            Component::ParentDir if out.has_root() => {}
            c => out.push(c),
        }
    }
    out
}

/// Every file Axentra OneDrive keeps, so tests and alternate profiles can relocate them.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_file: PathBuf,
    pub tokens_file: PathBuf,
    pub db_file: PathBuf,
    /// Downloaded file contents ("locally available" files).
    pub cache_dir: PathBuf,
    /// The freedesktop thumbnail cache (`~/.cache/thumbnails`), where cloud
    /// thumbnails go.
    pub thumbnail_dir: PathBuf,
}

impl Paths {
    pub fn user() -> Self {
        let config = dirs::config_dir().unwrap_or_else(|| expand_home("~/.config"));
        let data = dirs::data_dir().unwrap_or_else(|| expand_home("~/.local/share"));
        let cache = dirs::cache_dir().unwrap_or_else(|| expand_home("~/.cache"));
        Self {
            config_file: config.join("axentra/onedrive/config.toml"),
            tokens_file: data.join("axentra/onedrive/tokens.json"),
            db_file: data.join("axentra/onedrive/state.sqlite"),
            cache_dir: cache.join("axentra/onedrive/files"),
            thumbnail_dir: cache.join("thumbnails"),
        }
    }

    pub fn under(root: &Path) -> Self {
        Self {
            config_file: root.join("config.toml"),
            tokens_file: root.join("tokens.json"),
            db_file: root.join("state.sqlite"),
            cache_dir: root.join("files"),
            thumbnail_dir: root.join("thumbnails"),
        }
    }

    pub fn ensure(&self) -> Result<()> {
        for p in [&self.config_file, &self.tokens_file, &self.db_file] {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
        }
        std::fs::create_dir_all(&self.cache_dir)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_colour_defaults_to_ice() {
        let mut cfg = Config::default();
        assert_eq!(cfg.icon_color(), "ice");
        cfg.icon_color = "azure".into();
        assert_eq!(cfg.icon_color(), "azure");
        cfg.icon_color = "chartreuse".into();
        assert_eq!(cfg.icon_color(), "ice");
        // Older settings files without the key still load.
        let old: Config = toml::from_str("client_id = \"x\"").unwrap();
        assert_eq!(old.icon_color(), "ice");
        assert!(old.cloud_thumbnails);
        assert_eq!(icon_accent("periwinkle"), [139, 151, 255]);
    }

    #[test]
    fn paths_are_worked_out_like_the_shell() {
        let n = |p: &str| normalize(Path::new(p)).display().to_string();
        assert_eq!(
            n("/home/u/OneDrive/Pictures/2024/.."),
            "/home/u/OneDrive/Pictures"
        );
        assert_eq!(
            n("/home/u/Downloads/../OneDrive/./x/"),
            "/home/u/OneDrive/x"
        );
        assert_eq!(n("/.."), "/");
        assert_eq!(n("a/../../b/../.."), "../..");

        let home = dirs::home_dir().unwrap_or_default();
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("~/OneDrive"), home.join("OneDrive"));
        assert_eq!(expand_home("/srv/OneDrive"), PathBuf::from("/srv/OneDrive"));
        let relative = Config {
            mount_point: "OneDrive2/".into(),
            ..Default::default()
        };
        assert_eq!(relative.mount_path(), home.join("OneDrive2"));
    }

    #[test]
    fn a_broken_settings_file_needs_the_user() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "client_id = \"x\"\npoll_interval_secs = \"often\"\n").unwrap();
        let e = Config::load(&path).unwrap_err();
        let why = &e.downcast_ref::<NeedsUser>().unwrap().0;
        assert!(why.contains("on line 2"), "{why}");
        // A missing file is fine: it's the defaults.
        assert_eq!(
            Config::load(&dir.path().join("none.toml")).unwrap(),
            Config::default()
        );
    }
}
