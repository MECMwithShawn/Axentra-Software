//! JSON shapes the daemon serves over D-Bus to the tray, flyout and file
//! manager extension.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Status {
    /// starting, signed_out, up_to_date, syncing, paused, offline, error
    pub state: String,
    /// Headline as the flyout shows it, e.g. "Your files are synced".
    pub message: String,
    /// Second line, e.g. "Uploading 3 files · 12.4 MB of 80 MB".
    pub detail: String,
    pub uploading: u32,
    pub downloading: u32,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// Local changes not yet on OneDrive.
    pub pending: u64,
    pub issues: Vec<Issue>,
    /// Unix seconds.
    pub paused_until: Option<i64>,
    pub last_sync: Option<i64>,
    pub mount_point: String,
    pub transfers: Vec<Transfer>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Transfer {
    pub name: String,
    pub upload: bool,
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub signed_in: bool,
    pub name: String,
    pub email: String,
    /// "personal" or "business".
    pub drive_type: String,
    pub used: u64,
    pub total: u64,
    pub remaining: u64,
    /// normal, nearing, critical, exceeded
    pub quota_state: String,
    pub web_url: String,
}

impl Account {
    /// "OneDrive - Personal" or "OneDrive - <organisation>", as the Windows
    /// client titles its flyout.
    pub fn title(&self) -> String {
        match self.drive_type.as_str() {
            "business" => "OneDrive - Work or school".into(),
            _ => "OneDrive - Personal".into(),
        }
    }
}

/// Human-readable size, matching the Windows client ("1.2 GB", "340 KB").
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if v >= 100.0 {
        format!("{:.0} {}", v, UNITS[unit])
    } else {
        format!("{:.1} {}", v, UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes() {
        assert_eq!(super::human_size(512), "512 bytes");
        assert_eq!(super::human_size(1536), "1.5 KB");
        assert_eq!(super::human_size(5 * 1024 * 1024 * 1024), "5.0 GB");
        assert_eq!(super::human_size(1024 * 1024 * 1024 * 1024 + 1), "1.0 TB");
    }
}
