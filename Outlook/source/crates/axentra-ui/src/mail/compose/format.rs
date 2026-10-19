//! Formatting data for the compose body: fonts, sizes, colour palettes,
//! paragraph styles and the editor's reported selection state.

use serde::Deserialize;

/// Fonts offered in the Font box. Aptos is the new Outlook default; it is
/// not installable on Linux, so it renders with metric-compatible fallbacks
/// (see `font_stack`).
pub const FONTS: &[&str] = &[
    "Aptos",
    "Arial",
    "Calibri",
    "Cambria",
    "Comic Sans MS",
    "Courier New",
    "DejaVu Sans",
    "Georgia",
    "Liberation Sans",
    "Liberation Serif",
    "Noto Sans",
    "Noto Serif",
    "Segoe UI",
    "Tahoma",
    "Times New Roman",
    "Trebuchet MS",
    "Ubuntu",
    "Verdana",
];

/// Font sizes in points (mail spec 6.4).
pub const SIZES: &[f32] = &[8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 36.0, 48.0, 72.0];

/// CSS stack for a font name (same rules as the store's default body font).
pub fn font_stack(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    match lower.as_str() {
        "" | "aptos" => "Aptos, Calibri, Carlito, 'Noto Sans', sans-serif".into(),
        "calibri" => "Calibri, Carlito, sans-serif".into(),
        _ => {
            let generic = match lower.as_str() {
                "times new roman" | "georgia" | "cambria" | "liberation serif" | "noto serif" => "serif",
                "courier new" => "monospace",
                "comic sans ms" => "cursive",
                _ => "sans-serif",
            };
            if name.contains(' ') { format!("'{}', {generic}", name.trim()) } else { format!("{}, {generic}", name.trim()) }
        }
    }
}

/// "12" or "10.5".
pub fn size_label(pt: f32) -> String {
    if pt.fract() == 0.0 { format!("{}", pt as i32) } else { format!("{pt}") }
}

/// Next size up or down the list from `current`.
pub fn step_size(current: f32, up: bool) -> f32 {
    if up {
        SIZES.iter().copied().find(|s| *s > current + 0.01).unwrap_or(SIZES[SIZES.len() - 1])
    } else {
        SIZES.iter().rev().copied().find(|s| *s < current - 0.01).unwrap_or(SIZES[0])
    }
}

/// Office "Standard Colors" row (user data colours, not UI colours).
pub const STANDARD_COLORS: &[(&str, &str)] = &[
    ("#C00000", "Dark Red"),
    ("#FF0000", "Red"),
    ("#FFC000", "Orange"),
    ("#FFFF00", "Yellow"),
    ("#92D050", "Light Green"),
    ("#00B050", "Green"),
    ("#00B0F0", "Light Blue"),
    ("#0070C0", "Blue"),
    ("#002060", "Dark Blue"),
    ("#7030A0", "Purple"),
];

/// Highlight colours (the Office highlighter set).
pub const HIGHLIGHTS: &[(&str, &str)] = &[
    ("#FFFF00", "Yellow"),
    ("#00FF00", "Bright Green"),
    ("#00FFFF", "Turquoise"),
    ("#FF00FF", "Pink"),
    ("#0000FF", "Blue"),
    ("#FF0000", "Red"),
    ("#000080", "Dark Blue"),
    ("#008080", "Teal"),
    ("#008000", "Green"),
    ("#800080", "Violet"),
    ("#800000", "Dark Red"),
    ("#808000", "Dark Yellow"),
    ("#808080", "Gray 50%"),
    ("#C0C0C0", "Gray 25%"),
    ("#000000", "Black"),
];

const THEME_BASE: &[(&str, &str)] = &[
    ("#FFFFFF", "White"),
    ("#000000", "Black"),
    ("#E7E6E6", "Light Gray"),
    ("#44546A", "Blue-Gray"),
    ("#4472C4", "Blue"),
    ("#ED7D31", "Orange"),
    ("#A5A5A5", "Gray"),
    ("#FFC000", "Gold"),
    ("#5B9BD5", "Light Blue"),
    ("#70AD47", "Green"),
];

fn parse_hex(hex: &str) -> (f32, f32, f32) {
    let h = hex.trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f32;
    (c(0), c(2), c(4))
}

fn mix(hex: &str, toward: f32, amount: f32) -> String {
    let (r, g, b) = parse_hex(hex);
    let m = |c: f32| (c + (toward - c) * amount).round().clamp(0.0, 255.0) as u8;
    format!("#{:02X}{:02X}{:02X}", m(r), m(g), m(b))
}

/// The 10 x 6 theme palette (base row, then lighter and darker variants),
/// as (hex, name) in row-major order.
pub fn theme_palette() -> Vec<(String, String)> {
    let mut rows: Vec<Vec<(String, String)>> = vec![Vec::new(); 6];
    for (hex, name) in THEME_BASE {
        let variants: [(f32, f32, &str); 5] = match *name {
            "White" => [(0.0, 0.05, "Darker 5%"), (0.0, 0.15, "Darker 15%"), (0.0, 0.25, "Darker 25%"), (0.0, 0.35, "Darker 35%"), (0.0, 0.5, "Darker 50%")],
            "Black" => [(255.0, 0.5, "Lighter 50%"), (255.0, 0.35, "Lighter 35%"), (255.0, 0.25, "Lighter 25%"), (255.0, 0.15, "Lighter 15%"), (255.0, 0.05, "Lighter 5%")],
            _ => [(255.0, 0.8, "Lighter 80%"), (255.0, 0.6, "Lighter 60%"), (255.0, 0.4, "Lighter 40%"), (0.0, 0.25, "Darker 25%"), (0.0, 0.5, "Darker 50%")],
        };
        rows[0].push((hex.to_string(), name.to_string()));
        for (i, (toward, amount, label)) in variants.iter().enumerate() {
            rows[i + 1].push((mix(hex, *toward, *amount), format!("{name}, {label}")));
        }
    }
    rows.into_iter().flatten().collect()
}

/// Paragraph styles in the Styles gallery: (id, label, block tag).
pub const STYLES: &[(&str, &str, &str)] = &[
    ("normal", "Normal", "div"),
    ("h1", "Heading 1", "h1"),
    ("h2", "Heading 2", "h2"),
    ("h3", "Heading 3", "h3"),
    ("quote", "Quote", "blockquote"),
    ("code", "Code", "pre"),
];

pub const LINE_SPACINGS: &[&str] = &["1.0", "1.15", "1.5", "2.0"];

/// What the editor reports about the current selection.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct FormatState {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub sub: bool,
    pub sup: bool,
    pub ul: bool,
    pub ol: bool,
    /// "left", "center", "right" or "justify".
    pub align: String,
    pub font: String,
    pub size: f32,
    pub link: bool,
    /// Block tag of the caret: "div", "h1", "h2", "h3", "blockquote", "pre".
    pub block: String,
    /// "ltr" or "rtl".
    pub dir: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_step_through_the_list() {
        assert_eq!(step_size(12.0, true), 14.0);
        assert_eq!(step_size(12.0, false), 11.0);
        assert_eq!(step_size(13.0, false), 12.0);
        assert_eq!(step_size(72.0, true), 72.0);
        assert_eq!(step_size(8.0, false), 8.0);
        assert_eq!(size_label(10.5), "10.5");
        assert_eq!(size_label(12.0), "12");
    }

    #[test]
    fn theme_palette_is_ten_by_six() {
        let p = theme_palette();
        assert_eq!(p.len(), 60);
        assert_eq!(p[0], ("#FFFFFF".to_string(), "White".to_string()));
        assert_eq!(p[10].1, "White, Darker 5%");
        assert_eq!(p[10].0, "#F2F2F2");
        assert_eq!(p[11].0, "#808080");
    }

    #[test]
    fn font_stacks() {
        assert_eq!(font_stack("Aptos"), "Aptos, Calibri, Carlito, 'Noto Sans', sans-serif");
        assert_eq!(font_stack("Times New Roman"), "'Times New Roman', serif");
        assert_eq!(font_stack("Verdana"), "Verdana, sans-serif");
    }
}
