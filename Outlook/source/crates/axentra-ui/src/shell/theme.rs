//! Colour tokens, generated from one data table (docs/spec/shell.md §1, §12.1).
//!
//! Layers: the palette (`--ax-brand-*` ramp, `--ax-grey-*` in tokens.css) →
//! Fluent alias tokens (`--colorNeutralForeground1`…, light and dark values
//! below) → the `--ax-*` names modules use (mapped in tokens.css). A colour
//! theme only swaps the brand ramp; dark mode and high contrast only swap the
//! alias layer. Switching never re-renders the tree: the shell sets
//! attributes on `.ax-app` and these rules do the rest.

use std::sync::OnceLock;

/// Accessibility preferences the desktop reports (XDG portal, GNOME
/// settings). `None` = unknown; the CSS media queries decide then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemPrefs {
    pub high_contrast: Option<bool>,
    pub reduce_motion: Option<bool>,
}

static SYSTEM_PREFS: OnceLock<SystemPrefs> = OnceLock::new();

/// Called once by the desktop launcher before the window opens.
pub fn set_system_prefs(prefs: SystemPrefs) {
    let _ = SYSTEM_PREFS.set(prefs);
}

pub fn system_prefs() -> SystemPrefs {
    SYSTEM_PREFS.get().copied().unwrap_or_default()
}

/// `data-contrast` / `data-motion` value for a "follow system" setting:
/// the desktop's answer when known, else "system" (CSS media queries).
pub fn follow_system(setting: Option<bool>, system: Option<bool>, on: &'static str, off: &'static str) -> &'static str {
    match setting.or(system) {
        Some(true) => on,
        Some(false) => off,
        None => "system",
    }
}

/// (alias, light, dark). Values: `bN` = brand ramp step, `gN` = grey ramp
/// step, anything else is literal CSS.
const ALIASES: &[(&str, &str, &str)] = &[
    // Foregrounds
    ("colorNeutralForeground1", "g14", "#ffffff"),
    ("colorNeutralForeground1Hover", "g14", "#ffffff"),
    ("colorNeutralForeground2", "g26", "g84"),
    ("colorNeutralForeground2Hover", "g14", "#ffffff"),
    ("colorNeutralForeground2Pressed", "g14", "#ffffff"),
    ("colorNeutralForeground2Selected", "g14", "#ffffff"),
    ("colorNeutralForeground2BrandHover", "b80", "b100"),
    ("colorNeutralForeground2BrandPressed", "b70", "b90"),
    ("colorNeutralForeground2BrandSelected", "b80", "b100"),
    ("colorNeutralForeground3", "g38", "g68"),
    ("colorNeutralForeground3Hover", "g26", "g84"),
    ("colorNeutralForeground3Pressed", "g26", "g84"),
    ("colorNeutralForeground3Selected", "g26", "g84"),
    ("colorNeutralForeground4", "g44", "g60"),
    ("colorNeutralForegroundDisabled", "g74", "g36"),
    ("colorNeutralForegroundOnBrand", "#ffffff", "#ffffff"),
    ("colorNeutralForegroundInverted", "#ffffff", "g14"),
    ("colorNeutralForegroundInverted2", "#ffffff", "g14"),
    ("colorBrandForeground1", "b80", "b100"),
    ("colorBrandForeground2", "b70", "b110"),
    ("colorBrandForegroundLink", "b70", "b100"),
    ("colorBrandForegroundLinkHover", "b60", "b110"),
    ("colorBrandForegroundLinkPressed", "b40", "b90"),
    ("colorCompoundBrandForeground1", "b80", "b100"),
    ("colorCompoundBrandForeground1Hover", "b70", "b110"),
    ("colorCompoundBrandForeground1Pressed", "b60", "b90"),
    // Backgrounds
    ("colorNeutralBackground1", "#ffffff", "g16"),
    ("colorNeutralBackground1Hover", "g96", "g24"),
    ("colorNeutralBackground1Pressed", "g88", "g12"),
    ("colorNeutralBackground1Selected", "g92", "g22"),
    ("colorNeutralBackground2", "g98", "g12"),
    ("colorNeutralBackground2Hover", "g94", "g20"),
    ("colorNeutralBackground2Pressed", "g86", "g8"),
    ("colorNeutralBackground2Selected", "g90", "g18"),
    ("colorNeutralBackground3", "g96", "g8"),
    ("colorNeutralBackground3Hover", "g92", "g16"),
    ("colorNeutralBackground3Pressed", "g84", "g4"),
    ("colorNeutralBackground3Selected", "g88", "g14"),
    ("colorNeutralBackground4", "g94", "g4"),
    ("colorNeutralBackground5", "g92", "#000000"),
    ("colorNeutralBackground6", "g90", "g20"),
    ("colorNeutralBackgroundInverted", "g16", "#ffffff"),
    ("colorNeutralBackgroundDisabled", "g94", "g8"),
    ("colorSubtleBackground", "transparent", "transparent"),
    ("colorSubtleBackgroundHover", "g96", "g22"),
    ("colorSubtleBackgroundPressed", "g88", "g18"),
    ("colorSubtleBackgroundSelected", "g92", "g20"),
    ("colorBrandBackground", "b80", "b70"),
    ("colorBrandBackgroundHover", "b70", "b80"),
    ("colorBrandBackgroundPressed", "b40", "b40"),
    ("colorBrandBackgroundSelected", "b60", "b60"),
    ("colorBrandBackground2", "b160", "b20"),
    ("colorBrandBackground2Hover", "b150", "b40"),
    ("colorBrandBackground2Pressed", "b130", "b10"),
    ("colorCompoundBrandBackground", "b80", "b100"),
    ("colorCompoundBrandBackgroundHover", "b70", "b110"),
    ("colorCompoundBrandBackgroundPressed", "b60", "b90"),
    ("colorNeutralStencil1", "g90", "g34"),
    ("colorNeutralStencil2", "g98", "g20"),
    ("colorBackgroundOverlay", "rgba(0, 0, 0, 0.4)", "rgba(0, 0, 0, 0.5)"),
    // Strokes
    ("colorNeutralStroke1", "g82", "g40"),
    ("colorNeutralStroke1Hover", "g78", "g46"),
    ("colorNeutralStroke1Pressed", "g70", "g42"),
    ("colorNeutralStroke1Selected", "g74", "g44"),
    ("colorNeutralStroke2", "g88", "g32"),
    ("colorNeutralStroke3", "g94", "g24"),
    ("colorNeutralStrokeAccessible", "g38", "g68"),
    ("colorNeutralStrokeAccessibleHover", "g34", "g74"),
    ("colorNeutralStrokeAccessibleSelected", "b80", "b100"),
    ("colorNeutralStrokeDisabled", "g88", "g26"),
    ("colorBrandStroke1", "b80", "b100"),
    ("colorBrandStroke2", "b140", "b50"),
    ("colorCompoundBrandStroke", "b80", "b100"),
    ("colorCompoundBrandStrokeHover", "b70", "b110"),
    ("colorCompoundBrandStrokePressed", "b60", "b90"),
    ("colorStrokeFocus1", "#ffffff", "#000000"),
    ("colorStrokeFocus2", "#000000", "#ffffff"),
    ("colorTransparentStroke", "transparent", "transparent"),
    // Shadow colours
    ("colorNeutralShadowAmbient", "rgba(0, 0, 0, 0.12)", "rgba(0, 0, 0, 0.24)"),
    ("colorNeutralShadowKey", "rgba(0, 0, 0, 0.14)", "rgba(0, 0, 0, 0.28)"),
    ("colorNeutralShadowAmbientLighter", "rgba(0, 0, 0, 0.06)", "rgba(0, 0, 0, 0.12)"),
    ("colorNeutralShadowKeyLighter", "rgba(0, 0, 0, 0.07)", "rgba(0, 0, 0, 0.14)"),
    ("colorNeutralShadowAmbientDarker", "rgba(0, 0, 0, 0.2)", "rgba(0, 0, 0, 0.4)"),
    ("colorNeutralShadowKeyDarker", "rgba(0, 0, 0, 0.24)", "rgba(0, 0, 0, 0.48)"),
    ("colorBrandShadowAmbient", "rgba(0, 0, 0, 0.3)", "rgba(0, 0, 0, 0.3)"),
    ("colorBrandShadowKey", "rgba(0, 0, 0, 0.25)", "rgba(0, 0, 0, 0.25)"),
    // Status (§1.5)
    ("colorStatusDangerBackground1", "#fdf3f4", "#3b0509"),
    ("colorStatusDangerBackground2", "#eeacb2", "#6e0811"),
    ("colorStatusDangerBackground3", "#c50f1f", "#c50f1f"),
    ("colorStatusDangerBackground3Hover", "#b10e1c", "#b10e1c"),
    ("colorStatusDangerBackground3Pressed", "#960b18", "#960b18"),
    ("colorStatusDangerForeground1", "#b10e1c", "#dc626d"),
    ("colorStatusDangerForeground2", "#6e0811", "#eeacb2"),
    ("colorStatusDangerForeground3", "#c50f1f", "#eeacb2"),
    ("colorStatusDangerForegroundInverted", "#dc626d", "#b10e1c"),
    ("colorStatusDangerBorder1", "#eeacb2", "#c50f1f"),
    ("colorStatusDangerBorder2", "#c50f1f", "#dc626d"),
    ("colorStatusDangerBorderActive", "#c50f1f", "#dc626d"),
    ("colorStatusSuccessBackground1", "#f1faf1", "#052505"),
    ("colorStatusSuccessBackground2", "#9fd89f", "#094509"),
    ("colorStatusSuccessBackground3", "#107c10", "#107c10"),
    ("colorStatusSuccessForeground1", "#0e700e", "#54b054"),
    ("colorStatusSuccessForeground2", "#094509", "#9fd89f"),
    ("colorStatusSuccessForeground3", "#107c10", "#9fd89f"),
    ("colorStatusSuccessForegroundInverted", "#54b054", "#0e700e"),
    ("colorStatusSuccessBorder1", "#9fd89f", "#107c10"),
    ("colorStatusSuccessBorder2", "#107c10", "#9fd89f"),
    ("colorStatusSuccessBorderActive", "#107c10", "#54b054"),
    ("colorStatusWarningBackground1", "#fff9f5", "#4a1e04"),
    ("colorStatusWarningBackground2", "#fdcfb4", "#8a3707"),
    ("colorStatusWarningBackground3", "#f7630c", "#f7630c"),
    ("colorStatusWarningForeground1", "#bc4b09", "#faa06b"),
    ("colorStatusWarningForeground2", "#8a3707", "#fdcfb4"),
    ("colorStatusWarningForeground3", "#bc4b09", "#f98845"),
    ("colorStatusWarningForegroundInverted", "#faa06b", "#bc4b09"),
    ("colorStatusWarningBorder1", "#fdcfb4", "#f7630c"),
    ("colorStatusWarningBorder2", "#bc4b09", "#f98845"),
    ("colorStatusWarningBorderActive", "#f7630c", "#faa06b"),
    // Axentra app-semantic colours that differ by mode (§1.6, §3.1, §3.2)
    ("--ax-header-bg", "b80", "g12"),
    ("--ax-header-fg", "#ffffff", "#ffffff"),
    ("--ax-header-icon", "#ffffff", "g84"),
    ("--ax-header-icon-hover", "#ffffff", "#ffffff"),
    ("--ax-header-button-hover", "b70", "g22"),
    ("--ax-header-button-pressed", "b40", "g18"),
    ("--ax-header-button-selected", "b60", "g20"),
    ("--ax-header-search-bg", "rgba(255, 255, 255, 0.92)", "g16"),
    ("--ax-header-search-bg-hover", "#ffffff", "g16"),
    ("--ax-header-search-fg", "g14", "#ffffff"),
    ("--ax-header-search-placeholder", "g38", "g68"),
    ("--ax-header-focus-ring", "#ffffff", "#ffffff"),
    ("--ax-header-focus-inner", "#000000", "#000000"),
    ("--ax-flag", "#c50f1f", "#dc626d"),
    ("--ax-flagged-row-bg", "#fffad6", "#282400"),
    ("--ax-offline-pill-bg", "g16", "#ffffff"),
    ("--ax-offline-pill-fg", "#ffffff", "g14"),
    ("--ax-cal-elsewhere-stripe", "rgba(255, 255, 255, 0.6)", "rgba(0, 0, 0, 0.35)"),
];

/// Header colours of the Neutral theme in light mode (dark mode keeps the
/// neutral dark header of every theme).
const NEUTRAL_HEADER: &[(&str, &str)] = &[
    ("--ax-header-bg", "g96"),
    ("--ax-header-fg", "g14"),
    ("--ax-header-icon", "g26"),
    ("--ax-header-icon-hover", "g14"),
    ("--ax-header-button-hover", "g92"),
    ("--ax-header-button-pressed", "g84"),
    ("--ax-header-button-selected", "g88"),
    ("--ax-header-search-bg", "#ffffff"),
    ("--ax-header-search-bg-hover", "#ffffff"),
    ("--ax-header-focus-ring", "#000000"),
    ("--ax-header-focus-inner", "#ffffff"),
];

/// Fluent `brandWeb`, the default "Outlook blue" ramp (§1.2), steps 10..=160.
pub const BRAND_WEB: [&str; 16] = [
    "#061724", "#082338", "#0a2e4a", "#0c3b5e", "#0e4775", "#0f548c", "#115ea3", "#0f6cbd", "#2886de", "#479ef5",
    "#62abf5", "#77b7f7", "#96c6fa", "#b4d6fa", "#cfe4fa", "#ebf3fc",
];

/// A colour theme from Settings > General > Appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorTheme {
    pub id: &'static str,
    pub label: &'static str,
    /// Ramp step 80; `None` uses `brandWeb`.
    pub key: Option<&'static str>,
}

/// Colour-only themes (§1.15). Never Microsoft artwork.
pub const COLOR_THEMES: &[ColorTheme] = &[
    ColorTheme { id: "blue", label: "Blue", key: None },
    ColorTheme { id: "classic_blue", label: "Classic blue", key: Some("#0078d4") },
    ColorTheme { id: "neutral", label: "Neutral", key: None },
    ColorTheme { id: "teal", label: "Teal", key: Some("#038387") },
    ColorTheme { id: "purple", label: "Purple", key: Some("#5c2e91") },
    ColorTheme { id: "pink", label: "Pink", key: Some("#b4009e") },
    ColorTheme { id: "red", label: "Red", key: Some("#c4314b") },
    ColorTheme { id: "orange", label: "Orange", key: Some("#bc4b09") },
    ColorTheme { id: "green", label: "Green", key: Some("#107c10") },
    ColorTheme { id: "graphite", label: "Graphite", key: Some("#4f5b62") },
];

pub fn color_theme(id: &str) -> &'static ColorTheme {
    COLOR_THEMES.iter().find(|t| t.id == id).unwrap_or(&COLOR_THEMES[0])
}

/// Swatch colour shown for a theme in the picker.
pub fn swatch(theme: &ColorTheme) -> &'static str {
    theme.key.unwrap_or(if theme.id == "neutral" { "#e6e6e6" } else { BRAND_WEB[7] })
}

// ---------------------------------------------------------------- colour maths

/// Parse `#rrggbb`.
pub fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn to_hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_linear(c: f64) -> f64 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// sRGB → OKLCH (L 0..1, C, hue radians).
fn oklch((r, g, b): (u8, u8, u8)) -> (f64, f64, f64) {
    let (r, g, b) = (to_linear(r as f64 / 255.0), to_linear(g as f64 / 255.0), to_linear(b as f64 / 255.0));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    let ll = 0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s;
    let a = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
    let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
    (ll, (a * a + bb * bb).sqrt(), bb.atan2(a))
}

/// OKLCH → linear sRGB, unclamped (out-of-gamut values fall outside 0..1).
fn oklch_to_linear(ll: f64, c: f64, h: f64) -> [f64; 3] {
    let (a, b) = (c * h.cos(), c * h.sin());
    let l = (ll + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m = (ll - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s = (ll - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s,
    ]
}

/// OKLCH → sRGB, reducing chroma until the colour fits the gamut.
fn from_oklch(ll: f64, mut c: f64, h: f64) -> (u8, u8, u8) {
    let mut rgb = oklch_to_linear(ll, c, h);
    while c > 0.0 && rgb.iter().any(|v| !(-0.0001..=1.0001).contains(v)) {
        c = (c - 0.002).max(0.0);
        rgb = oklch_to_linear(ll, c, h);
    }
    let to8 = |v: f64| (from_linear(v.clamp(0.0, 1.0)) * 255.0).round() as u8;
    (to8(rgb[0]), to8(rgb[1]), to8(rgb[2]))
}

/// WCAG relative luminance.
pub fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
    0.2126 * to_linear(r as f64 / 255.0) + 0.7152 * to_linear(g as f64 / 255.0) + 0.0722 * to_linear(b as f64 / 255.0)
}

/// WCAG contrast ratio of two colours.
pub fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// A 16-step brand ramp (steps 10..=160) with `key` as step 80, interpolated
/// in OKLCH towards near-black at step 10 and near-white at step 160 (the
/// Fluent theme-designer approach).
pub fn generate_ramp(key: &str) -> Vec<String> {
    let Some(rgb) = parse_hex(key) else { return BRAND_WEB.iter().map(|s| s.to_string()).collect() };
    let (lk, ck, h) = oklch(rgb);
    (1..=16)
        .map(|i| {
            let step = i * 10;
            if step == 80 {
                return to_hex(rgb);
            }
            let (l, c) = if step < 80 {
                let t = (80 - step) as f64 / 70.0;
                (lk + (0.19 - lk) * t, ck * (1.0 - 0.62 * t))
            } else {
                let t = (step - 80) as f64 / 80.0;
                (lk + (0.965 - lk) * t, ck * (1.0 - 0.86 * t))
            };
            to_hex(from_oklch(l, c, h))
        })
        .collect()
}

// ---------------------------------------------------------------- CSS

fn value(v: &str) -> String {
    let num = |s: &str| s.parse::<u32>().ok();
    if let Some(n) = v.strip_prefix('b').and_then(num) {
        format!("var(--ax-brand-{n})")
    } else if let Some(n) = v.strip_prefix('g').and_then(num) {
        format!("var(--ax-grey-{n})")
    } else {
        v.to_string()
    }
}

fn prop(name: &str) -> String {
    if name.starts_with("--") { name.to_string() } else { format!("--{name}") }
}

fn block(selector: &str, decls: impl Iterator<Item = (String, String)>) -> String {
    let mut out = format!("{selector}{{");
    for (k, v) in decls {
        out.push_str(&k);
        out.push(':');
        out.push_str(&v);
        out.push(';');
    }
    out.push_str("}\n");
    out
}

fn mode_decls(dark: bool) -> impl Iterator<Item = (String, String)> {
    ALIASES.iter().map(move |(n, l, d)| (prop(n), value(if dark { d } else { l })))
}

/// High-contrast role of an alias: one of the system colour roles (§20.2).
fn hc_role(name: &str) -> &'static str {
    let has = |s: &str| name.contains(s);
    if has("Shadow") || has("Transparent") || name == "colorSubtleBackground" {
        return "transparent";
    }
    if has("Overlay") {
        return "overlay";
    }
    if has("Disabled") {
        return "GrayText";
    }
    if has("Link") {
        return "LinkText";
    }
    if name == "colorStrokeFocus2" || name == "--ax-header-focus-ring" {
        return "Highlight";
    }
    if name == "colorStrokeFocus1" || name == "--ax-header-focus-inner" {
        return "Canvas";
    }
    if has("OnBrand") {
        return "HighlightText";
    }
    if has("Foreground") || name.ends_with("-fg") || has("icon") || has("placeholder") || name == "--ax-flag" {
        if (has("Brand") || has("Compound")) && !has("Status") {
            return "Highlight";
        }
        return "CanvasText";
    }
    if has("Stroke") || has("Border") {
        if has("Brand") || has("Compound") || has("AccessibleSelected") {
            return "Highlight";
        }
        return "CanvasText";
    }
    if (name.starts_with("colorBrandBackground") && !has("Background2")) || name.starts_with("colorCompoundBrandBackground") {
        return "Highlight";
    }
    "Canvas"
}

/// High-contrast alias values: hex for the explicit setting, CSS system
/// colours under `forced-colors`.
fn hc_decls(dark: bool, system_colors: bool) -> impl Iterator<Item = (String, String)> {
    let palette = move |role: &str| -> String {
        if system_colors || role == "transparent" {
            return match role {
                "overlay" => "rgba(0, 0, 0, 0.5)".into(),
                r => r.to_string(),
            };
        }
        let (canvas, text, hi, hi_text, link, gray) = if dark {
            ("#000000", "#ffffff", "#1aebff", "#000000", "#ffff00", "#3ff23f")
        } else {
            ("#ffffff", "#000000", "#37006e", "#ffffff", "#00009f", "#600000")
        };
        match role {
            "Canvas" => canvas,
            "CanvasText" => text,
            "Highlight" => hi,
            "HighlightText" => hi_text,
            "LinkText" => link,
            "GrayText" => gray,
            _ => "rgba(0, 0, 0, 0.5)",
        }
        .to_string()
    };
    ALIASES.iter().map(move |(n, _, _)| (prop(n), palette(hc_role(n))))
}

const SYSTEM_DARK: &str = "@media (prefers-color-scheme: dark)";

/// All generated colour CSS: light/dark alias maps, theme ramps, the
/// Neutral theme header and high contrast.
pub fn theme_css(custom_accent: Option<&str>) -> String {
    let mut css = String::new();
    // Light (default), dark, and "Use system settings" when the OS prefers dark.
    css.push_str(&block(".ax-app,.ax-surface-light", mode_decls(false)));
    css.push_str(&block(".ax-app[data-color-theme=\"neutral\"]", NEUTRAL_HEADER.iter().map(|(n, v)| (n.to_string(), value(v)))));
    css.push_str(&block(".ax-app.ax-theme-dark", mode_decls(true)));
    css.push_str(&format!("{SYSTEM_DARK}{{{}}}\n", block(".ax-app.ax-theme-system", mode_decls(true))));
    // The light-surface helper keeps light values inside dark mode.
    css.push_str(&block(".ax-app.ax-theme-dark .ax-surface-light", mode_decls(false)));
    css.push_str(&format!("{SYSTEM_DARK}{{{}}}\n", block(".ax-app.ax-theme-system .ax-surface-light", mode_decls(false))));

    // Colour themes swap the brand ramp.
    let ramp_block = |sel: &str, ramp: &[String]| {
        block(sel, ramp.iter().enumerate().map(|(i, hex)| (format!("--ax-brand-{}", (i + 1) * 10), hex.clone())))
    };
    for theme in COLOR_THEMES {
        if let Some(key) = theme.key {
            css.push_str(&ramp_block(&format!(".ax-app[data-color-theme=\"{}\"]", theme.id), &generate_ramp(key)));
        }
    }
    if let Some(accent) = custom_accent.filter(|a| parse_hex(a).is_some()) {
        css.push_str(&ramp_block(".ax-app[data-color-theme=\"custom\"]", &generate_ramp(accent)));
    }

    // High contrast: explicit setting, OS "more contrast", forced colours.
    css.push_str(&block(".ax-app[data-contrast=\"high\"]", hc_decls(false, false)));
    css.push_str(&block(".ax-app.ax-theme-dark[data-contrast=\"high\"]", hc_decls(true, false)));
    css.push_str(&format!("{SYSTEM_DARK}{{{}}}\n", block(".ax-app.ax-theme-system[data-contrast=\"high\"]", hc_decls(true, false))));
    css.push_str(&format!(
        "@media (prefers-contrast: more){{{}{}}}\n",
        block(".ax-app[data-contrast=\"system\"]", hc_decls(false, false)),
        block(".ax-app.ax-theme-dark[data-contrast=\"system\"]", hc_decls(true, false)),
    ));
    css.push_str(&format!(
        "@media (prefers-contrast: more) and (prefers-color-scheme: dark){{{}}}\n",
        block(".ax-app.ax-theme-system[data-contrast=\"system\"]", hc_decls(true, false))
    ));
    css.push_str(&format!("@media (forced-colors: active){{{}}}\n", block(".ax-app:not([data-contrast=\"normal\"])", hc_decls(false, true))));
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_system_prefers_the_setting_then_the_desktop() {
        assert_eq!(follow_system(Some(true), Some(false), "high", "normal"), "high");
        assert_eq!(follow_system(None, Some(true), "high", "normal"), "high");
        assert_eq!(follow_system(None, Some(false), "reduce", "normal"), "normal");
        assert_eq!(follow_system(None, None, "high", "normal"), "system");
    }

    #[test]
    fn default_ramp_matches_brand_web() {
        let generated = generate_ramp(BRAND_WEB[7]);
        assert_eq!(generated[7], BRAND_WEB[7]);
        assert_eq!(generated.len(), 16);
    }

    #[test]
    fn ramps_keep_the_key_and_get_lighter() {
        for theme in COLOR_THEMES.iter().filter(|t| t.key.is_some()) {
            let ramp = generate_ramp(theme.key.unwrap());
            assert_eq!(ramp[7], theme.key.unwrap(), "{} keeps its key colour at step 80", theme.id);
            let lum: Vec<f64> = ramp.iter().map(|h| luminance(parse_hex(h).unwrap())).collect();
            assert!(lum.windows(2).all(|w| w[0] <= w[1] + 1e-9), "{} ramp is monotonic: {ramp:?}", theme.id);
            assert!(lum[0] < 0.03 && lum[15] > 0.8, "{} spans dark to light: {ramp:?}", theme.id);
        }
    }

    #[test]
    fn white_on_step_80_is_readable_for_every_theme() {
        // §1.15: white text on the step-80 header/primary button needs 4.5:1.
        for theme in COLOR_THEMES {
            let bg = parse_hex(theme.key.unwrap_or(BRAND_WEB[7])).unwrap();
            let ratio = contrast((255, 255, 255), bg);
            assert!(ratio >= 4.5, "{} has {ratio:.2}:1", theme.id);
        }
    }

    #[test]
    fn spec_alias_values_resolve() {
        let find = |n: &str| ALIASES.iter().find(|(a, _, _)| *a == n).copied().unwrap();
        assert_eq!(find("colorNeutralForeground1"), ("colorNeutralForeground1", "g14", "#ffffff"));
        assert_eq!(value("b80"), "var(--ax-brand-80)");
        assert_eq!(value("g16"), "var(--ax-grey-16)");
        assert_eq!(value("#ffffff"), "#ffffff");
        assert_eq!(hc_role("colorNeutralForeground1"), "CanvasText");
        assert_eq!(hc_role("colorBrandBackground"), "Highlight");
        assert_eq!(hc_role("colorNeutralForegroundOnBrand"), "HighlightText");
        assert_eq!(hc_role("colorNeutralForegroundDisabled"), "GrayText");
        assert_eq!(hc_role("colorBrandBackground2"), "Canvas");
    }

    #[test]
    fn css_covers_every_mode() {
        let css = theme_css(Some("#7a3e9d"));
        for needle in [
            ".ax-app.ax-theme-dark{",
            "@media (prefers-color-scheme: dark){.ax-app.ax-theme-system{",
            ".ax-app[data-color-theme=\"teal\"]{--ax-brand-10:",
            ".ax-app[data-color-theme=\"custom\"]",
            ".ax-app[data-contrast=\"high\"]{",
            "@media (forced-colors: active)",
            "--colorNeutralForeground1:var(--ax-grey-14);",
        ] {
            assert!(css.contains(needle), "missing {needle}");
        }
    }
}
