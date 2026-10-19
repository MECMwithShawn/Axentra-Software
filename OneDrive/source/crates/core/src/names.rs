//! OneDrive naming rules, ignore patterns and conflict copies.

use globset::{Glob, GlobSet, GlobSetBuilder};

const INVALID_CHARS: &[char] = &['"', '*', ':', '<', '>', '?', '/', '\\', '|'];
const RESERVED: &[&str] = &[
    ".lock",
    "CON",
    "PRN",
    "AUX",
    "NUL",
    "COM0",
    "COM1",
    "COM2",
    "COM3",
    "COM4",
    "COM5",
    "COM6",
    "COM7",
    "COM8",
    "COM9",
    "LPT0",
    "LPT1",
    "LPT2",
    "LPT3",
    "LPT4",
    "LPT5",
    "LPT6",
    "LPT7",
    "LPT8",
    "LPT9",
    "desktop.ini",
];

/// Why OneDrive would refuse this name, if it would.
pub fn invalid_reason(name: &str) -> Option<String> {
    if let Some(c) = name.chars().find(|c| INVALID_CHARS.contains(c)) {
        return Some(format!(
            "OneDrive doesn't allow the character '{c}' in names"
        ));
    }
    if name.starts_with(' ') || name.ends_with(' ') {
        return Some("OneDrive doesn't allow names that start or end with a space".into());
    }
    if name.ends_with('.') {
        return Some("OneDrive doesn't allow names that end with a period".into());
    }
    let stem = name.split('.').next().unwrap_or(name);
    if RESERVED
        .iter()
        .any(|r| r.eq_ignore_ascii_case(name) || r.eq_ignore_ascii_case(stem))
    {
        return Some(format!("\"{name}\" is a reserved name on OneDrive"));
    }
    if name.contains("_vti_") {
        return Some("OneDrive doesn't allow \"_vti_\" in names".into());
    }
    if name.len() > 255 {
        return Some("The name is longer than OneDrive allows".into());
    }
    None
}

/// GNOME/GIO trash folders. Refused at the mount root, so deleting from the
/// file manager falls back to a permanent local delete, which OneDrive turns
/// into a move to its online recycle bin.
pub fn is_trash_dir(name: &str) -> bool {
    name == ".Trash" || name.starts_with(".Trash-")
}

pub struct IgnoreRules(GlobSet);

impl IgnoreRules {
    pub fn new(patterns: &[String]) -> Self {
        let mut b = GlobSetBuilder::new();
        for p in patterns {
            match Glob::new(p) {
                Ok(g) => {
                    b.add(g);
                }
                Err(e) => tracing::warn!("ignoring bad ignore pattern {p:?}: {e}"),
            }
        }
        Self(b.build().unwrap_or_else(|_| GlobSet::empty()))
    }

    pub fn matches(&self, name: &str) -> bool {
        self.0.is_match(name)
    }
}

pub fn hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "linux".into())
}

/// The Windows client keeps both copies of a conflicting file by appending the
/// computer name: `Budget.xlsx` becomes `Budget-DESKTOP-1A2B.xlsx`.
pub fn conflict_name(name: &str, host: &str, attempt: u32) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    if attempt <= 1 {
        format!("{stem}-{host}{ext}")
    } else {
        format!("{stem}-{host}-{attempt}{ext}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_names_follow_windows_pattern() {
        assert_eq!(
            conflict_name("Budget.xlsx", "zorin", 1),
            "Budget-zorin.xlsx"
        );
        assert_eq!(
            conflict_name("Budget.xlsx", "zorin", 2),
            "Budget-zorin-2.xlsx"
        );
        assert_eq!(conflict_name("Makefile", "zorin", 1), "Makefile-zorin");
        assert_eq!(conflict_name(".bashrc", "zorin", 1), ".bashrc-zorin");
    }

    #[test]
    fn invalid_names() {
        assert!(invalid_reason("a:b.txt").is_some());
        assert!(invalid_reason("CON.txt").is_some());
        assert!(invalid_reason("desktop.ini").is_some());
        assert!(invalid_reason(" lead").is_some());
        assert!(invalid_reason("ok name #1 100%.txt").is_none());
    }

    #[test]
    fn ignore_defaults() {
        let rules = IgnoreRules::new(&crate::config::Config::default().ignore);
        assert!(rules.matches("~$Report.docx"));
        assert!(rules.matches(".~lock.Report.odt#"));
        assert!(rules.matches(".notes.txt.swp"));
        assert!(!rules.matches("Report.docx"));
    }
}
