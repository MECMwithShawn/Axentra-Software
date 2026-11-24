//! The product name in the Slint UI matches `BRAND`, and the GNOME Shell
//! extension still recognises the panel window by its title.

use std::path::Path;

use axentra_onedrive_core::BRAND;

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The quoted value of the first line starting with `prefix`.
fn quoted_after(text: &str, prefix: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("no line starting with {prefix:?}"));
    line.split('"').nth(1).expect("a quoted value").to_string()
}

/// The panel's window title as flyout.slint sets it.
fn flyout_title() -> String {
    let flyout = read("ui/flyout.slint");
    let window = flyout
        .split("export component Flyout inherits Window {")
        .nth(1)
        .expect("Flyout window in flyout.slint");
    let title = window
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("title:"))
        .expect("Flyout has a title")
        .trim()
        .trim_end_matches(';');
    match title {
        "Theme.app-name" => {
            quoted_after(&read("ui/theme.slint"), "out property <string> app-name:")
        }
        literal => literal.trim_matches('"').to_string(),
    }
}

#[test]
fn slint_app_name_is_the_brand() {
    let theme = read("ui/theme.slint");
    assert_eq!(
        quoted_after(&theme, "out property <string> app-name:"),
        BRAND
    );
    assert_eq!(flyout_title(), BRAND);
}

#[test]
fn gnome_extension_matches_the_panel_title_old_and_new() {
    let js = read("../../extras/gnome-shell/axentra-onedrive@mecmwithshawn.github.io/extension.js");
    let line = js
        .lines()
        .find(|l| l.starts_with("const PANEL_TITLES = ["))
        .expect("PANEL_TITLES in extension.js");
    let titles: Vec<&str> = line.split('\'').skip(1).step_by(2).collect();
    assert!(titles.contains(&flyout_title().as_str()), "{titles:?}");
    // Builds before the rename title the panel "Axentra OneDrive".
    assert!(titles.contains(&"Axentra OneDrive"), "{titles:?}");
    // The app id it checks is still the unchanged xdg app id.
    assert!(js.contains("const APP_ID = 'axentra-onedrive';"));
}
