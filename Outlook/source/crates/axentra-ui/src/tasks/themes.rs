//! List themes: 12 colours (no photos, Axentra decision). Each colour is a
//! data colour with a light-theme and a dark-theme shade chosen to keep 4.5:1
//! contrast for the list title against the tinted canvas.

/// (id, label, light shade, dark shade).
pub const THEMES: [(&str, &str, &str, &str); 12] = [
    ("blue", "Blue", "#0f6cbd", "#479ef5"),
    ("indigo", "Indigo", "#4f52b2", "#9399ff"),
    ("purple", "Purple", "#7a3db8", "#c29bfa"),
    ("rose", "Rose", "#b3245f", "#f07aaa"),
    ("red", "Red", "#c50f1f", "#f1707b"),
    ("orange", "Orange", "#b54400", "#faa06b"),
    ("brown", "Brown", "#8e562e", "#d9a37a"),
    ("olive", "Olive", "#5c6b10", "#b8c95a"),
    ("green", "Green", "#107c10", "#5ec75e"),
    ("teal", "Teal", "#00756b", "#4fd1c1"),
    ("steel", "Steel", "#2b5d8c", "#83b4e3"),
    ("gray", "Gray", "#5c5c5c", "#c4c4c4"),
];

/// Light and dark shade of a theme id.
pub fn theme_colors(id: &str) -> Option<(&'static str, &'static str)> {
    THEMES.iter().find(|t| t.0 == id).map(|t| (t.2, t.3))
}

/// Inline style setting the theme custom properties used by `tasks.css`.
pub fn theme_style(id: Option<&str>) -> String {
    match id.and_then(theme_colors) {
        Some((light, dark)) => format!("--ax-todo-theme-l:{light};--ax-todo-theme-d:{dark}"),
        None => String::new(),
    }
}
