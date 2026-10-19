//! Light or dark at startup (docs/spec/shell.md §1.15 "Mode resolution"):
//! the saved theme, or for "Use system settings" the XDG portal's
//! `org.freedesktop.appearance color-scheme`, then GNOME's `color-scheme`
//! key, then a GTK theme name ending in "-dark". The window gets the
//! matching GTK preference (so the webview's `prefers-color-scheme` agrees)
//! and a matching background colour, so nothing flashes white on open.

use std::process::Command;

use axentra_core::ThemeMode;

/// App canvas (`colorNeutralBackground3`): grey96 light, grey8 dark.
pub const CANVAS_LIGHT: (u8, u8, u8, u8) = (0xf5, 0xf5, 0xf5, 255);
pub const CANVAS_DARK: (u8, u8, u8, u8) = (0x14, 0x14, 0x14, 255);

/// What the desktop prefers: `Some(true)` dark, `Some(false)` light,
/// `None` when it states no preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemScheme(pub Option<bool>);

/// The number in `gdbus call ... ReadOne` output such as `(<uint32 1>,)`
/// or `(<<uint32 1>>,)` (version-1 portals).
pub fn parse_portal_u32(out: &str) -> Option<u32> {
    let n = out.split("uint32").nth(1)?.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect::<String>();
    n.parse().ok()
}

/// `color-scheme`: 0 = no preference, 1 = prefer dark, 2 = prefer light;
/// anything else counts as no preference (portal spec).
pub fn parse_portal(out: &str) -> Option<SystemScheme> {
    Some(SystemScheme(match parse_portal_u32(out)? {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }))
}

/// A gsettings boolean (`true` / `false`).
pub fn parse_bool(out: &str) -> Option<bool> {
    match out.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Parse `gsettings get org.gnome.desktop.interface color-scheme`.
pub fn parse_gnome(out: &str) -> Option<SystemScheme> {
    match out.trim().trim_matches('\'') {
        "prefer-dark" => Some(SystemScheme(Some(true))),
        "prefer-light" => Some(SystemScheme(Some(false))),
        "default" => Some(SystemScheme(None)),
        _ => None,
    }
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Read one `org.freedesktop.appearance` key from the settings portal.
fn portal(key: &str) -> Option<String> {
    run(
        "gdbus",
        &[
            "call", "--session", "--timeout", "1", "--dest", "org.freedesktop.portal.Desktop", "--object-path",
            "/org/freedesktop/portal/desktop", "--method", "org.freedesktop.portal.Settings.ReadOne",
            "org.freedesktop.appearance", key,
        ],
    )
}

/// Ask the desktop which scheme it prefers.
pub fn system_scheme() -> SystemScheme {
    if let Some(s) = portal("color-scheme").as_deref().and_then(parse_portal) {
        return s;
    }
    if let Some(s) = run("gsettings", &["get", "org.gnome.desktop.interface", "color-scheme"]).as_deref().and_then(parse_gnome) {
        return s;
    }
    let gtk = std::env::var("GTK_THEME").unwrap_or_default().to_lowercase();
    SystemScheme((gtk.ends_with("-dark") || gtk.ends_with(":dark")).then_some(true))
}

/// High contrast and reduced motion as the desktop reports them (keyboard
/// spec §20.1, §21): the portal's `contrast` and `reduced-motion` keys, then
/// GNOME's `high-contrast` and `enable-animations`. These drive the
/// "Follow system" choices in Settings > General > Accessibility.
pub fn system_prefs() -> axentra_ui::shell::theme::SystemPrefs {
    let gsetting = |schema: &str, key: &str| run("gsettings", &["get", schema, key]).as_deref().and_then(parse_bool);
    let high_contrast = portal("contrast")
        .as_deref()
        .and_then(parse_portal_u32)
        .map(|v| v == 1)
        .or_else(|| gsetting("org.gnome.desktop.a11y.interface", "high-contrast"));
    let reduce_motion = portal("reduced-motion")
        .as_deref()
        .and_then(parse_portal_u32)
        .map(|v| v == 1)
        .or_else(|| gsetting("org.gnome.desktop.interface", "enable-animations").map(|on| !on));
    axentra_ui::shell::theme::SystemPrefs { high_contrast, reduce_motion }
}

/// Whether the window opens dark. "No preference" resolves to light.
pub fn is_dark(theme: ThemeMode, system: impl FnOnce() -> SystemScheme) -> bool {
    match theme {
        ThemeMode::Light => false,
        ThemeMode::Dark => true,
        ThemeMode::System => system().0.unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_replies() {
        assert_eq!(parse_portal("(<uint32 1>,)\n"), Some(SystemScheme(Some(true))));
        assert_eq!(parse_portal("(<<uint32 2>>,)"), Some(SystemScheme(Some(false))));
        assert_eq!(parse_portal("(<uint32 0>,)"), Some(SystemScheme(None)));
        assert_eq!(parse_portal("(<uint32 7>,)"), Some(SystemScheme(None)), "unknown values mean no preference");
        assert_eq!(parse_portal("Error: GDBus.Error"), None);
    }

    #[test]
    fn portal_numbers_and_booleans() {
        assert_eq!(parse_portal_u32("(<uint32 1>,)"), Some(1));
        assert_eq!(parse_portal_u32("Error: GDBus.Error:org.freedesktop.portal.Error.NotFound"), None);
        assert_eq!(parse_bool("true\n"), Some(true));
        assert_eq!(parse_bool("'x'"), None);
    }

    #[test]
    fn gnome_replies() {
        assert_eq!(parse_gnome("'prefer-dark'\n"), Some(SystemScheme(Some(true))));
        assert_eq!(parse_gnome("'default'"), Some(SystemScheme(None)));
        assert_eq!(parse_gnome("No such key"), None);
    }

    #[test]
    fn explicit_modes_skip_the_system_query() {
        assert!(is_dark(ThemeMode::Dark, || panic!("not asked")));
        assert!(!is_dark(ThemeMode::Light, || panic!("not asked")));
        assert!(is_dark(ThemeMode::System, || SystemScheme(Some(true))));
        assert!(!is_dark(ThemeMode::System, || SystemScheme(None)), "no preference resolves to light");
    }
}
