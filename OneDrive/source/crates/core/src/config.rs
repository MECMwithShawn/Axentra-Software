//! User settings (`~/.config/axentra/onedrive/config.toml`) and on-disk locations.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DBUS_NAME: &str = "io.github.mecmwithshawn.AxentraOneDrive";
pub const DBUS_PATH: &str = "/io/github/mecmwithshawn/AxentraOneDrive";
pub const DBUS_IFACE: &str = "io.github.mecmwithshawn.AxentraOneDrive1";

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
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("reading {}", path.display()))
            }
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

    pub fn mount_path(&self) -> PathBuf {
        expand_home(&self.mount_point)
    }

    pub fn auth(&self) -> axentra_onedrive_graph::AuthConfig {
        axentra_onedrive_graph::AuthConfig {
            client_id: self.client_id.clone(),
            tenant: self.tenant.clone(),
            authority: self.authority.clone(),
        }
    }
}

pub fn expand_home(p: &str) -> PathBuf {
    if p == "~" {
        return dirs::home_dir().unwrap_or_default();
    }
    match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(p),
    }
}

/// Every file Axentra OneDrive keeps, so tests and alternate profiles can relocate them.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_file: PathBuf,
    pub tokens_file: PathBuf,
    pub db_file: PathBuf,
    /// Downloaded file contents ("locally available" files).
    pub cache_dir: PathBuf,
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
        }
    }

    pub fn under(root: &Path) -> Self {
        Self {
            config_file: root.join("config.toml"),
            tokens_file: root.join("tokens.json"),
            db_file: root.join("state.sqlite"),
            cache_dir: root.join("files"),
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
