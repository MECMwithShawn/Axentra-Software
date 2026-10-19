//! Calendar colours (data colours, not theme tokens): the Fluent shared-colour
//! ramps Outlook uses for calendars, and the per-event CSS variables that let
//! `calendar.css` pick the light or dark variant.

use axentra_core::{Category, Occurrence};

/// One calendar colour: (label, primary, tint50 = light fill, shade30 = light
/// text and dark fill, tint30 = dark bar). Values from the Fluent palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swatch {
    pub name: &'static str,
    pub primary: &'static str,
    pub tint50: &'static str,
    pub shade30: &'static str,
    pub tint30: &'static str,
}

const fn sw(name: &'static str, primary: &'static str, tint50: &'static str, shade30: &'static str, tint30: &'static str) -> Swatch {
    Swatch { name, primary, tint50, shade30, tint30 }
}

/// Colour picker order: the Graph calendar colours, then Fluent extras.
pub const PALETTE: &[Swatch] = &[
    sw("Blue", "#0078d4", "#d0e7f8", "#004377", "#5caae5"),
    sw("Teal", "#00b7c3", "#cef3f5", "#00666d", "#58d3db"),
    sw("Green", "#107c10", "#c9eac9", "#094509", "#54b054"),
    sw("Orange", "#f7630c", "#fee5d7", "#8a3707", "#faa06b"),
    sw("Yellow", "#eaa300", "#fcefd3", "#835b00", "#f2c661"),
    sw("Red", "#d13438", "#f8dadb", "#751d1f", "#e37d80"),
    sw("Pink", "#e43ba6", "#fbddf0", "#80215d", "#ef85c8"),
    sw("Brown", "#8e562e", "#edded3", "#50301a", "#bb8f6f"),
    sw("Gray", "#69797e", "#e4e9ea", "#3b4447", "#a0adb2"),
    sw("Purple", "#7160e8", "#e7e4fb", "#3f3682", "#a79cf1"),
    sw("Cranberry", "#c50f1f", "#f6d1d5", "#6e0811", "#dc626d"),
    sw("Grape", "#881798", "#eaceef", "#4c0d55", "#b55fc1"),
    sw("Dark teal", "#038387", "#c7ebec", "#02494c", "#4cb4b7"),
    sw("Steel", "#005b70", "#c3e1e8", "#00333f", "#4496a9"),
    sw("Magenta", "#bf0077", "#f5cee6", "#6b0043", "#d957a8"),
];

/// Colours that mean "Blue" (the account default before a user picks one).
const BLUE_ALIASES: &[&str] = &["#0f6cbd", "auto", ""];

fn parse_hex(hex: &str) -> Option<(f64, f64, f64)> {
    let h = hex.trim().trim_start_matches('#');
    let h = if h.len() == 3 { h.chars().flat_map(|c| [c, c]).collect::<String>() } else { h.to_string() };
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(&h, 16).ok()?;
    Some((((v >> 16) & 0xff) as f64, ((v >> 8) & 0xff) as f64, (v & 0xff) as f64))
}

/// sRGB mix: `t` = share of `b` (0.0 = all `a`).
pub fn mix(a: &str, b: &str, t: f64) -> String {
    let (Some(x), Some(y)) = (parse_hex(a), parse_hex(b)) else { return a.to_string() };
    let m = |p: f64, q: f64| (p + (q - p) * t.clamp(0.0, 1.0)).round() as u8;
    format!("#{:02x}{:02x}{:02x}", m(x.0, y.0), m(x.1, y.1), m(x.2, y.2))
}

/// The palette entry for a colour, if it is one (case-insensitive).
pub fn swatch(hex: &str) -> Option<&'static Swatch> {
    let h = hex.trim().to_ascii_lowercase();
    if BLUE_ALIASES.contains(&h.as_str()) {
        return PALETTE.first();
    }
    PALETTE.iter().find(|s| s.primary.eq_ignore_ascii_case(&h))
}

/// Display name for a calendar colour ("Blue", or "Custom").
pub fn color_name(hex: &str) -> &'static str {
    swatch(hex).map(|s| s.name).unwrap_or("Custom")
}

/// Every role an event colour plays, in both themes.
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    pub primary: String,
    /// Light theme: block fill and title text.
    pub fill: String,
    pub text: String,
    /// Dark theme: block fill, colour bar and secondary text.
    pub dark_fill: String,
    pub dark_bar: String,
    pub dark_text2: String,
    /// Lighter stripe for the Tentative bar (light theme).
    pub tint40: String,
    /// Darker stripe for Tentative fills (dark theme).
    pub dark_stripe: String,
}

/// The ramp for a calendar or category colour: exact Fluent values for the
/// palette, derived by mixing for custom colours.
pub fn ramp(hex: &str) -> Ramp {
    match swatch(hex) {
        Some(s) => Ramp {
            primary: s.primary.to_string(),
            fill: s.tint50.to_ascii_lowercase(),
            text: s.shade30.to_string(),
            dark_fill: s.shade30.to_string(),
            dark_bar: s.tint30.to_string(),
            dark_text2: s.tint50.to_ascii_lowercase(),
            tint40: mix(s.primary, "#ffffff", 0.55),
            dark_stripe: mix(s.primary, "#000000", 0.4),
        },
        None => {
            let p = if parse_hex(hex).is_some() { hex.trim().to_ascii_lowercase() } else { "#69797e".to_string() };
            Ramp {
                fill: mix(&p, "#ffffff", 0.8),
                text: mix(&p, "#000000", 0.55),
                dark_fill: mix(&p, "#000000", 0.55),
                dark_bar: mix(&p, "#ffffff", 0.35),
                dark_text2: mix(&p, "#ffffff", 0.8),
                tint40: mix(&p, "#ffffff", 0.55),
                dark_stripe: mix(&p, "#000000", 0.4),
                primary: p,
            }
        }
    }
}

/// Inline CSS custom properties for an event element; `calendar.css` maps
/// them to fill / bar / text per theme and Show-as style.
pub fn css_vars(r: &Ramp) -> String {
    format!(
        "--c-pri:{};--c-fill:{};--c-text:{};--c-dfill:{};--c-dbar:{};--c-dtext2:{};--c-tint40:{};--c-dstripe:{};",
        r.primary, r.fill, r.text, r.dark_fill, r.dark_bar, r.dark_text2, r.tint40, r.dark_stripe
    )
}

/// The colour an occurrence is drawn in: its first category's colour when it
/// has one, otherwise its calendar's.
pub fn event_hex(occ: &Occurrence, categories: &[Category]) -> String {
    occ.event
        .categories
        .first()
        .and_then(|name| categories.iter().find(|c| &c.name == name))
        .map(|c| crate::kit::category_colors(&c.color).0.to_string())
        .unwrap_or_else(|| occ.color.clone())
}

/// CSS variables for an occurrence (see [`css_vars`]).
pub fn event_style(occ: &Occurrence, categories: &[Category]) -> String {
    css_vars(&ramp(&event_hex(occ, categories)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_and_lookup() {
        assert_eq!(mix("#000000", "#ffffff", 0.5), "#808080");
        assert_eq!(mix("#0078d4", "#ffffff", 0.0), "#0078d4");
        assert_eq!(mix("#abc", "#000", 1.0), "#000000");
        assert_eq!(swatch("#0078D4").map(|s| s.name), Some("Blue"));
        assert_eq!(swatch("#0f6cbd").map(|s| s.name), Some("Blue"));
        assert_eq!(color_name("#123456"), "Custom");
    }

    #[test]
    fn palette_and_custom_ramps() {
        let blue = ramp("#0078d4");
        assert_eq!((blue.fill.as_str(), blue.text.as_str(), blue.dark_bar.as_str()), ("#d0e7f8", "#004377", "#5caae5"));
        let custom = ramp("#336699");
        assert_eq!(custom.fill, mix("#336699", "#ffffff", 0.8));
        assert_eq!(custom.dark_fill, mix("#336699", "#000000", 0.55));
        assert!(css_vars(&custom).starts_with("--c-pri:#336699;"));
        // Garbage falls back to gray instead of producing broken CSS.
        assert_eq!(ramp("not a colour").primary, "#69797e");
    }
}
