//! CSS colour values as email uses them, and the dark-mode colour mapping.
//!
//! Parsing covers `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()`
//! (comma and space syntax, numbers or percentages, `/ alpha`),
//! `hsl()`/`hsla()` (any angle unit), `hwb()`, `oklab()`, `oklch()`, the 148
//! named colours and `transparent`. `currentColor`, `inherit`, system
//! colours and the like are not colours to map and parse as `None`. HTML
//! colour attributes (`bgcolor`, `<font color>`) are parsed the way browsers
//! do ([`parse_legacy`]).
//!
//! The mapping works in OKLCH: lightness is inverted where it has to be,
//! hue is kept and chroma softened a little. Text and background mappings
//! are continuous and monotone on each side of their threshold, and every
//! mapping is idempotent even after the result is written out as bytes and
//! read back, so colours that already suit a dark background (light text,
//! dark backgrounds) are fixed points and emails that ship their own dark
//! styles are left as they are.

/// An sRGB colour, channels and alpha in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Rgba {
    pub const fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Rgba { r: r as f64 / 255.0, g: g as f64 / 255.0, b: b as f64 / 255.0, a: 1.0 }
    }

    fn from_hex24(v: u32) -> Self {
        Self::rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// The channels rounded to bytes (alpha included).
    pub fn to_u8(self) -> [u8; 4] {
        let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [q(self.r), q(self.g), q(self.b), q(self.a)]
    }

    /// `#rrggbb` when opaque, else `rgba(r, g, b, a)`.
    pub fn to_css(self) -> String {
        let [r, g, b, _] = self.to_u8();
        if self.a >= 0.999 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            let a = format!("{:.3}", self.a.clamp(0.0, 1.0));
            let a = a.trim_end_matches('0').trim_end_matches('.');
            format!("rgba({r}, {g}, {b}, {})", if a.is_empty() { "0" } else { a })
        }
    }

    /// `#rrggbb`, alpha dropped (HTML colour attributes have none).
    pub fn to_hex(self) -> String {
        Rgba { a: 1.0, ..self }.to_css()
    }

    /// This colour painted over an opaque one.
    pub fn over(self, below: Rgba) -> Rgba {
        let a = self.a.clamp(0.0, 1.0);
        let mix = |t: f64, u: f64| t * a + u * (1.0 - a);
        Rgba { r: mix(self.r, below.r), g: mix(self.g, below.g), b: mix(self.b, below.b), a: 1.0 }
    }

    /// WCAG relative luminance.
    pub fn luminance(self) -> f64 {
        0.2126 * to_linear(self.r) + 0.7152 * to_linear(self.g) + 0.0722 * to_linear(self.b)
    }

    /// OKLab lightness, `0.0` black to `1.0` white.
    pub fn lightness(self) -> f64 {
        to_oklch(self).0
    }

    /// OKLCH chroma (0 for greys, about 0.1 and up for vivid colours).
    pub fn chroma(self) -> f64 {
        to_oklch(self).1
    }
}

/// WCAG contrast ratio of two opaque colours (1 to 21).
pub fn contrast(a: Rgba, b: Rgba) -> f64 {
    let (x, y) = (a.luminance(), b.luminance());
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_gamma(c: f64) -> f64 {
    if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// (L, C, h in radians).
fn to_oklch(c: Rgba) -> (f64, f64, f64) {
    let (r, g, b) = (to_linear(c.r), to_linear(c.g), to_linear(c.b));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    let ll = 0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s;
    let aa = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
    let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
    (ll, aa.hypot(bb), bb.atan2(aa))
}

/// Linear sRGB of an OKLab colour (may be out of gamut).
fn oklab_to_linear(l: f64, a: f64, b: f64) -> [f64; 3] {
    let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
    [
        4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_,
        -1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701 * s_,
    ]
}

/// An OKLCH colour in sRGB, chroma reduced until it fits the gamut.
fn from_oklch(l: f64, c: f64, h: f64, alpha: f64) -> Rgba {
    let l = l.clamp(0.0, 1.0);
    let at = |c: f64| oklab_to_linear(l, c * h.cos(), c * h.sin());
    let fits = |v: [f64; 3]| v.iter().all(|x| (-1e-4..=1.0 + 1e-4).contains(x));
    let mut v = at(c);
    if !fits(v) {
        let (mut lo, mut hi) = (0.0, c);
        for _ in 0..16 {
            let mid = (lo + hi) / 2.0;
            if fits(at(mid)) { lo = mid } else { hi = mid }
        }
        v = at(lo);
    }
    Rgba { r: to_gamma(v[0].clamp(0.0, 1.0)), g: to_gamma(v[1].clamp(0.0, 1.0)), b: to_gamma(v[2].clamp(0.0, 1.0)), a: alpha }
}

// ---- parsing ----

/// Parse a CSS colour value. `None` for anything that isn't a concrete
/// colour (`currentColor`, `inherit`, `var()`, system colours, garbage).
pub fn parse(value: &str) -> Option<Rgba> {
    // Out-of-range components (`oklab(7 1e308 -1e308)`) can overflow.
    parse_any(value).filter(|c| [c.r, c.g, c.b, c.a].iter().all(|v| v.is_finite()))
}

fn parse_any(value: &str) -> Option<Rgba> {
    let v = value.trim();
    if let Some(hex) = v.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(open) = v.find('(') {
        if !v.ends_with(')') {
            return None;
        }
        let name = v[..open].trim().to_ascii_lowercase();
        let args = &v[open + 1..v.len() - 1];
        return parse_function(&name, args);
    }
    let lower = v.to_ascii_lowercase();
    if lower == "transparent" {
        return Some(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 });
    }
    named(&lower).map(Rgba::from_hex24)
}

/// Parse an HTML colour attribute (`bgcolor`, `<font color>`) with the HTML
/// "rules for parsing a legacy colour value", as WebKit does: named colours
/// and `#rgb`, then any other text read as hex digits (`#fffff` is
/// `#fffff0`, `#FFFFFF;` is `#ffff00`, `FFFFFF` is white, `rgb(255,0,0)` is
/// `#002500`). `None` where the browser ignores the attribute (empty,
/// `transparent`). Always opaque.
pub fn parse_legacy(value: &str) -> Option<Rgba> {
    if value.is_empty() {
        return None;
    }
    let v = value.trim_matches(|c: char| c.is_ascii_whitespace());
    if v.eq_ignore_ascii_case("transparent") {
        return None;
    }
    if let Some(n) = named(&v.to_ascii_lowercase()) {
        return Some(Rgba::from_hex24(n));
    }
    if v.len() == 4 && v.starts_with('#') && v[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        return parse_hex(&v[1..]);
    }
    // Characters outside the BMP count as "00"; at most 128 characters.
    let mut digits: Vec<u8> = Vec::with_capacity(v.len().min(130));
    for c in v.chars() {
        if c as u32 > 0xffff {
            digits.extend_from_slice(b"00");
        } else {
            digits.push(if c.is_ascii_hexdigit() { c as u8 } else if c == '#' { b'#' } else { b'0' });
        }
        if digits.len() >= 128 {
            break;
        }
    }
    digits.truncate(128);
    if digits.first() == Some(&b'#') {
        digits.remove(0);
    }
    for d in &mut digits {
        if *d == b'#' {
            *d = b'0';
        }
    }
    while digits.is_empty() || !digits.len().is_multiple_of(3) {
        digits.push(b'0');
    }
    let mut len = digits.len() / 3;
    let mut parts: Vec<&[u8]> = digits.chunks(len).collect();
    if len > 8 {
        parts = parts.iter().map(|p| &p[len - 8..]).collect();
        len = 8;
    }
    while len > 2 && parts.iter().all(|p| p[0] == b'0') {
        parts = parts.iter().map(|p| &p[1..]).collect();
        len -= 1;
    }
    let byte = |p: &[u8]| u8::from_str_radix(std::str::from_utf8(&p[..len.min(2)]).ok()?, 16).ok();
    Some(Rgba::rgb8(byte(parts[0])?, byte(parts[1])?, byte(parts[2])?))
}

fn parse_hex(hex: &str) -> Option<Rgba> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|d| d * 17);
    let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    let (r, g, b, a) = match hex.len() {
        3 => (digit(0)?, digit(1)?, digit(2)?, 255),
        4 => (digit(0)?, digit(1)?, digit(2)?, digit(3)?),
        6 => (pair(0)?, pair(2)?, pair(4)?, 255),
        8 => (pair(0)?, pair(2)?, pair(4)?, pair(6)?),
        _ => return None,
    };
    Some(Rgba { a: a as f64 / 255.0, ..Rgba::rgb8(r, g, b) })
}

/// A number with an optional unit: (value, lowercase unit).
fn number(s: &str) -> Option<(f64, String)> {
    let s = s.trim();
    let b = s.as_bytes();
    let digits = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        i
    };
    let mut i = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    let int_end = digits(i);
    let mut end = int_end;
    if b.get(end) == Some(&b'.') {
        end = digits(end + 1);
    }
    if end == i || (end == i + 1 && int_end == i) {
        return None;
    }
    i = end;
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let j = i + 1 + usize::from(matches!(b.get(i + 1), Some(b'+' | b'-')));
        if b.get(j).is_some_and(u8::is_ascii_digit) {
            i = digits(j);
        }
    }
    let v: f64 = s[..i].parse().ok()?;
    v.is_finite().then(|| (v, s[i..].to_ascii_lowercase()))
}

/// Function arguments: (components, alpha). Comma syntax or space syntax
/// with `/ alpha`.
fn args(inner: &str) -> Option<(Vec<&str>, Option<&str>)> {
    if inner.contains(',') {
        let mut parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.iter().any(|p| p.is_empty() || p.contains('/')) {
            return None;
        }
        let alpha = if parts.len() == 4 { parts.pop() } else { None };
        return (parts.len() == 3).then_some((parts, alpha));
    }
    let (main, alpha) = match inner.split_once('/') {
        Some((m, a)) => (m, Some(a.trim())),
        None => (inner, None),
    };
    let parts: Vec<&str> = main.split_whitespace().collect();
    (parts.len() == 3 && alpha.is_none_or(|a| !a.is_empty() && !a.contains(char::is_whitespace))).then_some((parts, alpha))
}

/// A component: `none` is 0; percentages scale to `pct_scale`.
fn component(s: &str, pct_scale: f64) -> Option<f64> {
    if s.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    match number(s)? {
        (v, u) if u.is_empty() => Some(v),
        (v, u) if u == "%" => Some(v / 100.0 * pct_scale),
        _ => None,
    }
}

fn alpha(s: Option<&str>) -> Option<f64> {
    s.map_or(Some(1.0), |a| component(a, 1.0)).map(|a| a.clamp(0.0, 1.0))
}

/// Hue in degrees.
fn hue(s: &str) -> Option<f64> {
    if s.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    let (v, u) = number(s)?;
    let deg = match u.as_str() {
        "" | "deg" => v,
        "rad" => v.to_degrees(),
        "grad" => v * 0.9,
        "turn" => v * 360.0,
        _ => return None,
    };
    Some(deg.rem_euclid(360.0))
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    let f = |n: f64| {
        let k = (n + h / 30.0) % 12.0;
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    (f(0.0), f(8.0), f(4.0))
}

fn parse_function(name: &str, inner: &str) -> Option<Rgba> {
    let (c, a) = args(inner)?;
    let a = alpha(a)?;
    let rgba = |r: f64, g: f64, b: f64| Some(Rgba { r: r.clamp(0.0, 1.0), g: g.clamp(0.0, 1.0), b: b.clamp(0.0, 1.0), a });
    match name {
        "rgb" | "rgba" => {
            let ch = |s: &str| component(s, 255.0).map(|v| v / 255.0);
            rgba(ch(c[0])?, ch(c[1])?, ch(c[2])?)
        }
        "hsl" | "hsla" => {
            // Saturation and lightness: percentages, or plain numbers (CSS 4).
            let pct = |s: &str| component(s, 100.0).map(|v| v / 100.0);
            let (r, g, b) = hsl_to_rgb(hue(c[0])?, pct(c[1])?, pct(c[2])?);
            rgba(r, g, b)
        }
        "hwb" => {
            let pct = |s: &str| component(s, 100.0).map(|v| v / 100.0);
            let (h, w, bl) = (hue(c[0])?, pct(c[1])?.clamp(0.0, 1.0), pct(c[2])?.clamp(0.0, 1.0));
            if w + bl >= 1.0 {
                let grey = w / (w + bl);
                return rgba(grey, grey, grey);
            }
            let (r, g, b) = hsl_to_rgb(h, 1.0, 0.5);
            let k = 1.0 - w - bl;
            rgba(r * k + w, g * k + w, b * k + w)
        }
        "oklab" => {
            let l = component(c[0], 1.0)?;
            let (aa, bb) = (component(c[1], 0.4)?, component(c[2], 0.4)?);
            let v = oklab_to_linear(l.clamp(0.0, 1.0), aa, bb);
            rgba(to_gamma(v[0].clamp(0.0, 1.0)), to_gamma(v[1].clamp(0.0, 1.0)), to_gamma(v[2].clamp(0.0, 1.0)))
        }
        "oklch" => {
            let l = component(c[0], 1.0)?;
            let ch = component(c[1], 0.4)?.max(0.0);
            Some(from_oklch(l, ch, hue(c[2])?.to_radians(), a))
        }
        _ => None,
    }
}

/// The CSS named colours (sorted for binary search).
const NAMED: [(&str, u32); 148] = [
    ("aliceblue", 0xf0f8ff), ("antiquewhite", 0xfaebd7), ("aqua", 0x00ffff), ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff), ("beige", 0xf5f5dc), ("bisque", 0xffe4c4), ("black", 0x000000),
    ("blanchedalmond", 0xffebcd), ("blue", 0x0000ff), ("blueviolet", 0x8a2be2), ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887), ("cadetblue", 0x5f9ea0), ("chartreuse", 0x7fff00), ("chocolate", 0xd2691e),
    ("coral", 0xff7f50), ("cornflowerblue", 0x6495ed), ("cornsilk", 0xfff8dc), ("crimson", 0xdc143c),
    ("cyan", 0x00ffff), ("darkblue", 0x00008b), ("darkcyan", 0x008b8b), ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9), ("darkgreen", 0x006400), ("darkgrey", 0xa9a9a9), ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b), ("darkolivegreen", 0x556b2f), ("darkorange", 0xff8c00), ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000), ("darksalmon", 0xe9967a), ("darkseagreen", 0x8fbc8f), ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f), ("darkslategrey", 0x2f4f4f), ("darkturquoise", 0x00ced1), ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493), ("deepskyblue", 0x00bfff), ("dimgray", 0x696969), ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff), ("firebrick", 0xb22222), ("floralwhite", 0xfffaf0), ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff), ("gainsboro", 0xdcdcdc), ("ghostwhite", 0xf8f8ff), ("gold", 0xffd700),
    ("goldenrod", 0xdaa520), ("gray", 0x808080), ("green", 0x008000), ("greenyellow", 0xadff2f),
    ("grey", 0x808080), ("honeydew", 0xf0fff0), ("hotpink", 0xff69b4), ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082), ("ivory", 0xfffff0), ("khaki", 0xf0e68c), ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5), ("lawngreen", 0x7cfc00), ("lemonchiffon", 0xfffacd), ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080), ("lightcyan", 0xe0ffff), ("lightgoldenrodyellow", 0xfafad2), ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90), ("lightgrey", 0xd3d3d3), ("lightpink", 0xffb6c1), ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa), ("lightskyblue", 0x87cefa), ("lightslategray", 0x778899), ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de), ("lightyellow", 0xffffe0), ("lime", 0x00ff00), ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6), ("magenta", 0xff00ff), ("maroon", 0x800000), ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd), ("mediumorchid", 0xba55d3), ("mediumpurple", 0x9370db), ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee), ("mediumspringgreen", 0x00fa9a), ("mediumturquoise", 0x48d1cc), ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970), ("mintcream", 0xf5fffa), ("mistyrose", 0xffe4e1), ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead), ("navy", 0x000080), ("oldlace", 0xfdf5e6), ("olive", 0x808000),
    ("olivedrab", 0x6b8e23), ("orange", 0xffa500), ("orangered", 0xff4500), ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa), ("palegreen", 0x98fb98), ("paleturquoise", 0xafeeee), ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5), ("peachpuff", 0xffdab9), ("peru", 0xcd853f), ("pink", 0xffc0cb),
    ("plum", 0xdda0dd), ("powderblue", 0xb0e0e6), ("purple", 0x800080), ("rebeccapurple", 0x663399),
    ("red", 0xff0000), ("rosybrown", 0xbc8f8f), ("royalblue", 0x4169e1), ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072), ("sandybrown", 0xf4a460), ("seagreen", 0x2e8b57), ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d), ("silver", 0xc0c0c0), ("skyblue", 0x87ceeb), ("slateblue", 0x6a5acd),
    ("slategray", 0x708090), ("slategrey", 0x708090), ("snow", 0xfffafa), ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4), ("tan", 0xd2b48c), ("teal", 0x008080), ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347), ("turquoise", 0x40e0d0), ("violet", 0xee82ee), ("wheat", 0xf5deb3),
    ("white", 0xffffff), ("whitesmoke", 0xf5f5f5), ("yellow", 0xffff00), ("yellowgreen", 0x9acd32),
];

fn named(lower: &str) -> Option<u32> {
    NAMED.binary_search_by(|(n, _)| (*n).cmp(lower)).ok().map(|i| NAMED[i].1)
}

/// True for a name in the named-colour table (any case).
pub fn is_named(ident: &str) -> bool {
    named(&ident.to_ascii_lowercase()).is_some()
}

// ---- the dark mapping ----

/// What a colour paints, which decides how it is adapted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// `color`, `<font color>`, `-webkit-text-fill-color`.
    Text,
    /// `background(-color)`, `bgcolor`, gradient stops (and shadows).
    Background,
    /// Border and outline colours.
    Border,
}

/// What text or a border sits on: the email's own (light-mode) background
/// and what it became on the dark canvas, both opaque.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Backdrop {
    pub orig: Rgba,
    pub mapped: Rgba,
    /// The original is a guess (a translucent background over a page the
    /// HTML doesn't give, or backgrounds the adapter can't place): text
    /// that seems to match it is not taken as hidden or faded.
    pub guess: bool,
}

impl Backdrop {
    pub const fn new(orig: Rgba, mapped: Rgba) -> Self {
        Backdrop { orig, mapped, guess: false }
    }

    /// A light background left as designed on the dark canvas (an image's
    /// own backing, an accent on a dark design, the sender's own dark-mode
    /// plate): what sits on it keeps its colours.
    pub fn kept(&self) -> bool {
        self.orig.to_u8() == self.mapped.to_u8() && !is_dark(self.orig)
    }
}

/// The dark message canvas (`--ax-bg-1` dark, the frame's body).
pub const DARK_CANVAS: Rgba = Rgba::rgb8(0x29, 0x29, 0x29);
/// The frame's link colour on the dark canvas (`--ax-fg-link` dark).
pub const DARK_LINK: Rgba = Rgba::rgb8(0x47, 0x9e, 0xf5);
/// The frame's link colour on the light page (`--ax-fg-link` light).
pub const LIGHT_LINK: Rgba = Rgba::rgb8(0x11, 0x5e, 0xa3);
/// The frame's text colour on the light page (`--ax-fg-1` light): what
/// text with no colour of its own was designed in.
pub const LIGHT_TEXT: Rgba = Rgba::rgb8(0x24, 0x24, 0x24);
/// The light page an email is designed for.
pub const WHITE: Rgba = Rgba::rgb8(0xff, 0xff, 0xff);

/// Backgrounds lighter than this (OKLab L) are darkened; darker ones are
/// already dark-mode backgrounds and stay. Mapped backgrounds land in
/// `[BG_WHITE, BG_SPLIT]`, so the mapping is continuous here.
const BG_SPLIT: f64 = 0.50;
/// Where white lands: a step above the canvas (L 0.28), so a white card
/// keeps an edge against it.
const BG_WHITE: f64 = 0.305;
/// < 1 spreads the near-white end (white, #fafafa, #f4f4f4, #eee) apart so
/// rows and cards that differ slightly stay visibly different.
const BG_CURVE: f64 = 0.5;
/// Chroma kept at the light end (pastels get a little softer).
const BG_CHROMA: f64 = 0.8;
/// Translucent backgrounds below this alpha are tints over whatever is
/// underneath; they stay as they are.
pub const BG_MIN_ALPHA: f64 = 0.3;

/// Borders, measured by luminance level (`cbrt(Y)`, OKLab L for greys) so a
/// saturated blue counts as dark as it looks next to the canvas. Levels in
/// `[BORDER_LO, BORDER_HI]` stay; lighter borders are darkened into that
/// band (white to `BORDER_LO`: dividers stay subtle) and darker ones are
/// lifted into it (black to `BORDER_HI`: grid lines and rules stay
/// visible). Both sides land in the band, so the mapping is idempotent.
const BORDER_LO: f64 = 0.45;
const BORDER_HI: f64 = 0.70;

/// Text darker than this is inverted (the darker, the lighter); lighter text
/// only gets lifted to the contrast floor if it's below it.
const TEXT_SPLIT: f64 = 0.60;
/// Chroma kept for black-ish text (softened a little; mid tones keep more).
const TEXT_CHROMA: f64 = 0.85;
/// WCAG AA for body text.
pub const MIN_CONTRAST: f64 = 4.5;
/// Text at or below this contrast against its own background was hidden by
/// the sender (white on white); it stays hidden.
const HIDDEN_CONTRAST: f64 = 1.1;
/// Light text under this contrast on a light background was faded on
/// purpose (an empty rating star, a placeholder): it lands on the dimmest
/// colour that reads instead of staying at full brightness.
const FADED_CONTRAST: f64 = 1.75;
/// Inverted text above this lightness gives up chroma as it gets lighter
/// (a dark green or red doesn't come out neon).
const TEXT_TAPER: f64 = 0.72;

/// The colour as it reads back after being written out (`#rrggbb` or
/// `rgba()` with three decimals): what a second pass would see.
pub fn written(c: Rgba) -> Rgba {
    if c.a >= 0.999 {
        let [r, g, b, _] = c.to_u8();
        return Rgba::rgb8(r, g, b);
    }
    parse(&c.to_css()).unwrap_or(c)
}

/// True for a background that already suits the dark canvas (one the
/// mapping leaves alone).
pub fn is_dark(c: Rgba) -> bool {
    c.lightness() <= BG_SPLIT
}

/// The background text is checked against when the HTML gives no
/// background for it (it may come from a class in a style sheet): the
/// mapped colour of a light grey (#e8e8e8), a little lighter than mapped
/// white, so text clears 4.5:1 on white, #fafafa, #f4f4f4 and #eee boxes.
pub fn default_text_backdrop() -> Rgba {
    map_background(Rgba::rgb8(0xe8, 0xe8, 0xe8))
}

/// Adapt a background colour for the dark canvas.
pub fn map_background(c: Rgba) -> Rgba {
    if c.a < BG_MIN_ALPHA {
        return c;
    }
    let (l, ch, h) = to_oklch(c);
    if l <= BG_SPLIT {
        return c;
    }
    // 0 for white, 1 at the split.
    let d = ((1.0 - l) / (1.0 - BG_SPLIT)).clamp(0.0, 1.0);
    let mut nl = BG_WHITE + (BG_SPLIT - BG_WHITE) * d.powf(BG_CURVE);
    let k = BG_CHROMA + (1.0 - BG_CHROMA) * d;
    let mut out = from_oklch(nl, ch * k, h, c.a);
    // Written out, it must still be at or under the split (a fixed point).
    for _ in 0..64 {
        if written(out).lightness() <= BG_SPLIT {
            break;
        }
        nl -= 0.001;
        out = from_oklch(nl, ch * k, h, c.a);
    }
    out
}

fn border_level(c: Rgba) -> f64 {
    c.luminance().cbrt()
}

/// True for a colour that [`map_border`] lifts or keeps (dark and mid
/// tones): as a rule or a dot on a light background, ink.
pub fn is_inky(c: Rgba) -> bool {
    border_level(c) <= BORDER_HI
}

/// Adapt a border colour for the dark canvas (for a border whose
/// background was light, or isn't known; see [`map`]).
pub fn map_border(c: Rgba) -> Rgba {
    if c.a < BG_MIN_ALPHA {
        return c;
    }
    let g = border_level(c);
    let (target, k) = if g > BORDER_HI {
        let d = ((1.0 - g) / (1.0 - BORDER_HI)).clamp(0.0, 1.0);
        (BORDER_LO + (BORDER_HI - BORDER_LO) * d.powf(1.5), BG_CHROMA + (1.0 - BG_CHROMA) * d)
    } else if g < BORDER_LO {
        let t = (1.0 - g / BORDER_LO).clamp(0.0, 1.0);
        (BORDER_LO + (BORDER_HI - BORDER_LO) * t.powf(0.3), 1.0)
    } else {
        return c;
    };
    let (_, ch, h) = to_oklch(c);
    let at = |l: f64| from_oklch(l, ch * k, h, c.a);
    // The OKLab lightness whose luminance level is the target.
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if border_level(at(mid)) < target { lo = mid } else { hi = mid }
    }
    let mut nl = hi;
    let mut out = at(nl);
    for _ in 0..64 {
        let g = border_level(written(out));
        if g > BORDER_HI {
            nl -= 0.001;
        } else if g < BORDER_LO {
            nl += 0.001;
        } else {
            break;
        }
        out = at(nl);
    }
    out
}

/// Adapt a text colour for the dark canvas, given the (already mapped,
/// opaque) background it sits on.
///
/// Light text that reads (4.5:1) stays. Dark text is inverted: black
/// becomes white and text just under the split lands on the lowest
/// lightness that reads on the backdrop, so the darker the original, the
/// lighter the result. Light text that doesn't quite read is lifted to that
/// floor. Output always reads and maps to itself, also once written out
/// and read back.
pub fn map_text(c: Rgba, backdrop: Rgba) -> Rgba {
    if c.a <= 0.0 {
        return c;
    }
    // A second pass sees the backdrop as written out.
    let backdrop = written(Rgba { a: 1.0, ..backdrop });
    let (l, ch, h) = to_oklch(c);
    let reads = |c: Rgba, min: f64| contrast(c.over(backdrop), backdrop) >= min;
    if l >= TEXT_SPLIT && reads(c, MIN_CONTRAST) {
        return c;
    }
    let inverted = (l < TEXT_SPLIT).then(|| 1.0 - l / TEXT_SPLIT);
    let ch = inverted.map_or(ch, |t| ch * (1.0 - (1.0 - TEXT_CHROMA) * t));
    // Inverted colours taper their chroma towards white.
    let taper = |nl: f64| if inverted.is_some() { 1.0 - 0.4 * ((nl - TEXT_TAPER) / (1.0 - TEXT_TAPER)).clamp(0.0, 1.0) } else { 1.0 };
    // The lowest lightness above the backdrop's that reads, with a little
    // headroom so byte rounding never drops the result under 4.5:1.
    let at = |nl: f64| from_oklch(nl, ch * taper(nl), h, c.a);
    let target = MIN_CONTRAST + 0.1;
    let floor = if !reads(at(1.0), target) {
        1.0
    } else {
        let (mut lo, mut hi) = (backdrop.lightness().min(1.0), 1.0);
        for _ in 0..20 {
            let mid = (lo + hi) / 2.0;
            if reads(at(mid), target) { hi = mid } else { lo = mid }
        }
        hi
    };
    let mut nl = match inverted {
        Some(t) => {
            let base = floor.max(TEXT_SPLIT);
            base + (1.0 - base) * t
        }
        None => floor.max(l),
    };
    let mut out = at(nl);
    // Written out and read back, it must still be a fixed point.
    let fixed = |x: Rgba| {
        let w = written(x);
        w.lightness() >= TEXT_SPLIT && reads(w, MIN_CONTRAST)
    };
    while !fixed(out) && nl < 1.0 {
        nl = (nl + 0.002).min(1.0);
        out = at(nl);
    }
    out
}

/// Adapt a colour for its role.
///
/// Text is checked against the background it sits on (unknown: against
/// [`default_text_backdrop`]); text the sender hid by matching its
/// background (white on white) takes the mapped background, so it stays
/// hidden, and light text the sender faded on a light background (an
/// empty rating star) lands on the dimmest colour that reads. A border on
/// a background that was already dark stays as designed (white outlines on
/// a dark hero); other borders go through [`map_border`]. On a background
/// kept light ([`Backdrop::kept`]) text and borders stay as designed.
pub fn map(c: Rgba, role: Role, backdrop: Option<Backdrop>) -> Rgba {
    match role {
        Role::Text => match backdrop {
            Some(b) if b.kept() => c,
            Some(b) if !b.guess && c.a > 0.0 && contrast(c.over(b.orig), b.orig) <= HIDDEN_CONTRAST => Rgba { a: c.a, ..b.mapped },
            Some(b) => {
                let t = if !b.guess && !is_dark(b.orig) && c.lightness() >= TEXT_SPLIT && contrast(c.over(b.orig), b.orig) < FADED_CONTRAST {
                    let (_, ch, h) = to_oklch(c);
                    map_text(from_oklch(TEXT_SPLIT, ch, h, c.a), b.mapped)
                } else {
                    map_text(c, b.mapped)
                };
                // Faint translucent text that still can't be told from its
                // background (a second pass would take it as hidden) is.
                let m = written(Rgba { a: 1.0, ..b.mapped });
                if !b.guess && c.a < 1.0 && contrast(t.over(m), m) <= HIDDEN_CONTRAST { Rgba { a: c.a, ..b.mapped } } else { t }
            }
            None => map_text(c, default_text_backdrop()),
        },
        Role::Background => map_background(c),
        Role::Border => match backdrop {
            Some(b) if is_dark(b.orig) || b.kept() => c,
            _ => map_border(c),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Rgba {
        parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    fn bytes(c: Option<Rgba>) -> Option<[u8; 4]> {
        c.map(Rgba::to_u8)
    }

    #[test]
    fn named_table_is_complete_and_sorted() {
        assert_eq!(NAMED.len(), 148);
        assert!(NAMED.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(bytes(parse("RebeccaPurple")), Some([0x66, 0x33, 0x99, 255]));
        assert_eq!(bytes(parse("WhiteSmoke")), Some([245, 245, 245, 255]));
        assert_eq!(bytes(parse("transparent")), Some([0, 0, 0, 0]));
        assert!(is_named("DarkSlateGray") && !is_named("solid") && !is_named("no-repeat"));
    }

    #[test]
    fn parses_every_form() {
        let cases: &[(&str, [u8; 4])] = &[
            ("#333", [0x33, 0x33, 0x33, 255]),
            ("#3338", [0x33, 0x33, 0x33, 0x88]),
            ("#1F497D", [0x1f, 0x49, 0x7d, 255]),
            ("#33333380", [0x33, 0x33, 0x33, 0x80]),
            ("rgb(51,51,51)", [51, 51, 51, 255]),
            ("rgb(51 51 51)", [51, 51, 51, 255]),
            ("RGB( 51 , 51 , 51 )", [51, 51, 51, 255]),
            ("rgba(0,0,0,.87)", [0, 0, 0, 222]),
            ("rgba(0, 0, 0, 87%)", [0, 0, 0, 222]),
            ("rgb(0 0 0 / 0.6)", [0, 0, 0, 153]),
            ("rgb(0 0 0 / 60%)", [0, 0, 0, 153]),
            ("rgb(20%, 20%, 20%)", [51, 51, 51, 255]),
            ("rgb(300, -5, 128)", [255, 0, 128, 255]),
            ("rgb(none 0 0)", [0, 0, 0, 255]),
            ("hsl(0, 0%, 20%)", [51, 51, 51, 255]),
            ("hsla(0, 0%, 20%, 1)", [51, 51, 51, 255]),
            ("hsl(0deg 0% 30% / 90%)", [77, 77, 77, 230]),
            ("hsl(120, 60%, 25%)", [26, 102, 26, 255]),
            ("hsl(210, 40%, 96%)", [241, 245, 249, 255]),
            ("hsl(0.5turn 100% 50%)", [0, 255, 255, 255]),
            ("hsl(3.14159rad 100% 50%)", [0, 255, 255, 255]),
            ("hsl(200grad 100% 50%)", [0, 255, 255, 255]),
            ("hsl(-120 100% 50%)", [0, 0, 255, 255]),
            ("hwb(0 0% 0%)", [255, 0, 0, 255]),
            ("hwb(0 60% 60%)", [128, 128, 128, 255]),
            ("oklab(1 0 0)", [255, 255, 255, 255]),
            ("oklch(0% 0 0)", [0, 0, 0, 255]),
            ("navy", [0, 0, 128, 255]),
            ("Black", [0, 0, 0, 255]),
            ("dimgray", [105, 105, 105, 255]),
        ];
        for (s, want) in cases {
            assert_eq!(bytes(parse(s)), Some(*want), "{s}");
        }
        for s in ["currentColor", "inherit", "initial", "unset", "windowtext", "var(--x)", "#12", "#12345", "#ggg", "rgb(1,2)", "rgb(1 2 3 4)", "rgb(1, 2 3)", "rgb(1px, 2, 3)", "lab(50 0 0)", "", "solid"] {
            assert_eq!(parse(s), None, "{s}");
        }
    }

    #[test]
    fn attributes_parse_like_the_browser() {
        // What WebKitGTK 2.52 renders for each bgcolor / <font color> value
        // (the HTML legacy colour rules, measured).
        let cases: &[(&str, [u8; 3])] = &[
            ("#fffff", [255, 255, 240]),
            ("#FFFFFF;", [255, 255, 0]),
            ("fff fff", [255, 15, 240]),
            ("#ffffffff", [255, 255, 255]),
            ("#ffffff80", [255, 255, 128]),
            ("white;", [0, 0, 224]),
            ("rgb(255,0,0)", [0, 37, 0]),
            ("hsl(0,100%,50%)", [0, 16, 80]),
            ("chucknorris", [192, 0, 0]),
            ("FFFFFF", [255, 255, 255]),
            ("fff", [15, 15, 15]),
            ("#fff", [255, 255, 255]),
            ("#ffff", [255, 255, 0]),
            ("window", [0, 13, 0]),
            ("currentcolor", [192, 224, 0]),
            (" #E1E1E1 ", [225, 225, 225]),
            ("#1234567890abc", [18, 103, 171]),
            ("#", [0, 0, 0]),
            ("#0g0", [0, 0, 0]),
            ("lightgoldenrodyellow", [250, 250, 210]),
            ("WHITE", [255, 255, 255]),
            ("#ffffff ", [255, 255, 255]),
            ("\u{e9}\u{e9}", [0, 0, 0]),
            ("rgba(0,0,0,0)", [186, 0, 0]),
            ("#zzz", [0, 0, 0]),
            ("😀ff", [0, 255, 0]),
        ];
        for (s, [r, g, b]) in cases {
            assert_eq!(bytes(parse_legacy(s)), Some([*r, *g, *b, 255]), "{s:?}");
        }
        // Ignored by the browser: nothing to adapt, the backdrop is the parent's.
        for s in ["", "transparent", " Transparent "] {
            assert_eq!(parse_legacy(s), None, "{s:?}");
        }
        // Long and odd input never panics.
        for s in ["#".repeat(300), "é".repeat(300), "😀".repeat(100), "0".repeat(1000)] {
            let _ = parse_legacy(&s);
        }
    }

    #[test]
    fn css_output() {
        assert_eq!(hex("#ABCDEF").to_css(), "#abcdef");
        assert_eq!(hex("rgba(1,2,3,0.5)").to_css(), "rgba(1, 2, 3, 0.5)");
        assert_eq!(hex("rgba(1,2,3,0.5)").to_hex(), "#010203");
        assert_eq!(hex("transparent").to_css(), "rgba(0, 0, 0, 0)");
    }

    #[test]
    fn dark_text_becomes_light_and_readable() {
        let canvas = default_text_backdrop();
        for s in ["#000", "#0F1111", "#222", "#24292f", "#333", "#3c3c3c", "#555", "#565959", "#666", "#757575", "#7F7F7F", "#888", "#1F497D", "#0563C1", "#0969da", "#B12704", "#007600", "#c62828", "#2e7d32", "navy", "darkred"] {
            let c = hex(s);
            let m = map_text(c, canvas);
            assert!(contrast(m, canvas) >= MIN_CONTRAST, "{s} -> {} {:.2}", m.to_css(), contrast(m, canvas));
            assert!(m.lightness() > 0.7, "{s} -> {}", m.to_css());
        }
        // Darker in, lighter out.
        assert!(map_text(hex("#000"), canvas).lightness() > map_text(hex("#333"), canvas).lightness());
        assert!(map_text(hex("#333"), canvas).lightness() > map_text(hex("#666"), canvas).lightness());
        // Hue is kept: blue links stay blue, red amounts stay red.
        let blue = map_text(hex("#0969da"), canvas).to_u8();
        assert!(blue[2] > blue[0] + 40, "{blue:?}");
        let red = map_text(hex("#c62828"), canvas).to_u8();
        assert!(red[0] > red[2] + 40, "{red:?}");
    }

    #[test]
    fn light_text_and_dark_backgrounds_stay() {
        let canvas = default_text_backdrop();
        for s in ["#ffffff", "#f5f5f7", "#e5e5e5", "#d6d6e0", "#cccccc", "#b7b7c4", "#7cf3ff"] {
            assert_eq!(map_text(hex(s), canvas), hex(s), "{s}");
        }
        for s in ["#000000", "#0d0d12", "#17171f", "#1e1e1e", "#232F3E", "#22222d", "#14213d", "#292929", "#444", "#555"] {
            assert_eq!(map_background(hex(s)), hex(s), "{s}");
        }
        // Faint tints stay tints.
        assert_eq!(map_background(hex("rgba(255,255,255,0.1)")), hex("rgba(255,255,255,0.1)"));
        assert_eq!(map_background(hex("transparent")), hex("transparent"));
    }

    #[test]
    fn light_backgrounds_become_dark_with_steps_kept() {
        let white = map_background(hex("#ffffff"));
        let page = map_background(hex("#f4f4f4"));
        let row = map_background(hex("#f6f6f6"));
        let grey = map_background(hex("#e0e0e0"));
        assert!(white.lightness() > DARK_CANVAS.lightness() + 0.015, "white {}", white.to_css());
        assert!(white.lightness() < 0.33);
        // Near-white steps stay visibly apart (inverted order).
        assert!(page.lightness() - white.lightness() > 0.03, "{} {}", white.to_css(), page.to_css());
        assert!(row.lightness() - white.lightness() > 0.03);
        assert!(grey.lightness() > page.lightness());
        for s in ["#ffffff", "#f6f8fa", "#FFD814", "#FFA41C", "yellow", "#e7f0ff", "#fff8e1", "#eef4fb", "#cfe2ff", "#aaaaaa"] {
            let m = map_background(hex(s));
            assert!(m.lightness() <= BG_SPLIT + 1e-6, "{s} -> {}", m.to_css());
            // White text reads on every mapped background.
            assert!(contrast(hex("#fff"), m) >= 6.0, "{s} -> {}", m.to_css());
        }
        // Mid-tone brand colours darken a little and keep their hue; white
        // text on them only gets easier to read.
        for s in ["#0078d4", "#0b5cff", "#cc0000", "#ff3d7f", "#6c2bd9", "#8f5cf0"] {
            let c = hex(s);
            let m = map_background(c);
            assert!(contrast(hex("#fff"), m) >= contrast(hex("#fff"), c), "{s}");
            assert!(contrast(hex("#fff"), m) >= MIN_CONTRAST, "{s} -> {}", m.to_css());
            let ([r0, _, b0, _], [r1, _, b1, _]) = (c.to_u8(), m.to_u8());
            assert_eq!(r0 > b0, r1 > b1, "{s} -> {}", m.to_css());
        }
        // Alpha is kept.
        assert_eq!(map_background(hex("rgba(255,255,255,0.9)")).a, hex("rgba(255,255,255,0.9)").a);
    }

    #[test]
    fn pairs_stay_readable() {
        // Text on its own (mapped) background: dark on light and light on
        // mid-tone pairs both read after mapping.
        for (fg, bg) in [("#0F1111", "#FFD814"), ("#0F1111", "#FFA41C"), ("#1d4ed8", "#e7f0ff"), ("#5d4037", "#fff8e1"), ("#ffffff", "#0078d4"), ("#ffffff", "#cc0000"), ("#000", "#3a9bd9"), ("#9a6700", "#fff8c5"), ("#116329", "#dafbe1"), ("#76768a", "#17171f")] {
            let b = map_background(hex(bg));
            let t = map_text(hex(fg), b);
            assert!(contrast(t, b) >= MIN_CONTRAST, "{fg} on {bg} -> {} on {} {:.2}", t.to_css(), b.to_css(), contrast(t, b));
        }
    }

    #[test]
    fn mapping_is_continuous_monotone_and_idempotent() {
        let canvas = default_text_backdrop();
        let greys: Vec<Rgba> = (0..=255u8).map(|v| Rgba::rgb8(v, v, v)).collect();
        // Text: non-increasing below the split (darker in, lighter out),
        // non-decreasing above it (up to the rounding headroom at the
        // contrast floor), and continuous across it.
        let text: Vec<f64> = greys.iter().map(|&g| map_text(g, canvas).lightness()).collect();
        let split = greys.iter().position(|g| g.lightness() >= TEXT_SPLIT).unwrap();
        assert!(text[..split].windows(2).all(|w| w[1] <= w[0] + 1e-9), "text below split");
        assert!(text[split..].windows(2).all(|w| w[1] >= w[0] - 0.01), "text above split: {:?}", &text[split..]);
        assert!((text[split] - text[split - 1]).abs() < 0.01, "text jumps at the split");
        assert!(text.iter().all(|&l| l >= TEXT_SPLIT), "all text ends up light");
        // Backgrounds: identity up to the split, non-increasing above it,
        // and continuous across it.
        let bg: Vec<f64> = greys.iter().map(|&g| map_background(g).lightness()).collect();
        let bsplit = greys.iter().position(|g| g.lightness() > BG_SPLIT).unwrap();
        assert!(bg[..bsplit].iter().zip(&greys).all(|(&m, g)| (m - g.lightness()).abs() < 1e-9), "bg below split");
        assert!(bg[bsplit..].windows(2).all(|w| w[1] <= w[0] + 1e-9), "bg above split");
        assert!((bg[bsplit] - bg[bsplit - 1]).abs() < 0.01, "bg jumps at the split");
        // Idempotent: mapping a mapped colour (as written out and read
        // back) changes nothing.
        let again = |c: Rgba| parse(&c.to_css()).unwrap();
        let samples = ["#000", "#333", "#666", "#7F7F7F", "#999", "#ccc", "#fff", "#1F497D", "#0969da", "#FFD814", "#e7f0ff", "#0078d4", "#c62828", "#f4f4f4", "rgba(0,0,0,.6)", "rgba(255,255,255,.9)", "#3338"];
        for s in samples {
            let c = hex(s);
            for backdrop in [canvas, DARK_CANVAS, map_background(hex("#FFD814")), hex("#17171f")] {
                let t = map_text(c, backdrop);
                assert_eq!(map_text(again(t), backdrop).to_u8(), again(t).to_u8(), "text {s} on {}", backdrop.to_css());
            }
            let b = map_background(c);
            assert_eq!(map_background(again(b)).to_u8(), again(b).to_u8(), "bg {s}");
            let r = map_border(c);
            assert_eq!(map_border(again(r)).to_u8(), again(r).to_u8(), "border {s}");
        }
    }

    #[test]
    fn borders_fold_into_a_visible_band() {
        let white = map_background(hex("#fff"));
        // Light dividers darken but stay subtle and visible.
        for s in ["#e5e5e5", "#d0d7de", "#E1E1E1", "#eeeeee", "#ffffff"] {
            let m = map_border(hex(s));
            assert!(m.lightness() < 0.56 && contrast(m, white) > 1.25, "{s} -> {}", m.to_css());
        }
        // Dark rules and grid lines are lifted (the darker, the lighter) and
        // stay visible on the canvas; hue is kept.
        let mut last = 0.0;
        for s in ["#444444", "#333333", "#222222", "#000000"] {
            let m = map_border(hex(s));
            assert!(contrast(m, DARK_CANVAS) >= 3.0 && m.lightness() > last, "{s} -> {}", m.to_css());
            last = m.lightness();
        }
        assert!(contrast(map_border(hex("#000")), DARK_CANVAS) > 5.0);
        // Mid-tone blue rules (an Apple Mail cite bar, a brand band) reach
        // 3:1, the minimum for lines.
        for s in ["#1010ff", "#0b4f8a"] {
            let m = map_border(hex(s));
            assert!(contrast(m, DARK_CANVAS) >= 3.0, "{s} -> {} {:.2}", m.to_css(), contrast(m, DARK_CANVAS));
        }
        let blue = map_border(hex("#1010ff"));
        let [r, _, b, _] = blue.to_u8();
        assert!(b > r + 60, "{}", blue.to_css());
        // Mid greys are already visible on both canvases.
        assert_eq!(map_border(hex("#999999")), hex("#999999"));
        // On a background that was dark, a border stays as designed.
        let dark = Backdrop::new(hex("#111111"), hex("#111111"));
        assert_eq!(map(hex("#ffffff"), Role::Border, Some(dark)), hex("#ffffff"));
        let light = Backdrop::new(WHITE, white);
        assert_ne!(map(hex("#ffffff"), Role::Border, Some(light)), hex("#ffffff"));
    }

    #[test]
    fn hidden_text_stays_hidden() {
        let white = Backdrop::new(WHITE, map_background(WHITE));
        // White on white (a hidden preheader) takes the mapped background.
        assert_eq!(map(hex("#ffffff"), Role::Text, Some(white)).to_u8(), white.mapped.to_u8());
        assert_eq!(map(hex("#fefefe"), Role::Text, Some(white)).to_u8(), white.mapped.to_u8());
        // Visible text on the same backdrop is adapted to read.
        let t = map(hex("#999999"), Role::Text, Some(white));
        assert!(contrast(t, white.mapped) >= MIN_CONTRAST);
        // White text on a dark background is not hidden text.
        let navy = Backdrop::new(hex("#0b1d3a"), hex("#0b1d3a"));
        assert_eq!(map(hex("#ffffff"), Role::Text, Some(navy)), hex("#ffffff"));
    }

    #[test]
    fn kept_and_guessed_backdrops() {
        // On a background left light (an accent on a dark design), text and
        // borders are as designed.
        let yellow = Backdrop::new(hex("#ffe600"), hex("#ffe600"));
        assert!(yellow.kept() && !Backdrop::new(hex("#111"), hex("#111")).kept());
        assert_eq!(map(hex("#000000"), Role::Text, Some(yellow)), hex("#000000"));
        assert_eq!(map(hex("#000000"), Role::Border, Some(yellow)), hex("#000000"));
        // Over a guessed page, white text is not taken as hidden.
        let white = Backdrop::new(WHITE, map_background(WHITE));
        let guess = Backdrop { guess: true, ..white };
        assert_eq!(map(hex("#ffffff"), Role::Text, Some(guess)), hex("#ffffff"));
        assert_eq!(map(hex("#ffffff"), Role::Text, Some(white)).to_u8(), white.mapped.to_u8());
    }

    #[test]
    fn faded_light_text_lands_on_the_dimmest_colour_that_reads() {
        let white = Backdrop::new(WHITE, map_background(WHITE));
        for s in ["#d2d2d2", "#cccccc", "#dddddd"] {
            let m = map(hex(s), Role::Text, Some(white));
            assert!(contrast(m, white.mapped) >= MIN_CONTRAST && contrast(m, white.mapped) < 5.2, "{s} -> {}", m.to_css());
            // A second pass (on the mapped background) keeps it.
            let again = parse(&m.to_css()).unwrap();
            assert_eq!(map(again, Role::Text, Some(Backdrop::new(white.mapped, white.mapped))).to_u8(), again.to_u8());
        }
        // Light grey that was meant to read (#999 on white, 2.8:1) is not faded.
        let m = map(hex("#999999"), Role::Text, Some(white));
        assert!(m.lightness() > map(hex("#d2d2d2"), Role::Text, Some(white)).lightness() - 0.2);
        assert!(contrast(map(hex("#bbbbbb"), Role::Text, Some(white)), white.mapped) > 5.2);
    }

    #[test]
    fn inverted_vivid_text_tapers_its_chroma() {
        let canvas = default_text_backdrop();
        for s in ["#004d00", "#006400", "#7a0019"] {
            let (c, m) = (hex(s), map_text(hex(s), canvas));
            assert!(m.lightness() > TEXT_TAPER && m.chroma() < 0.85 * c.chroma(), "{s} -> {} C {:.3} vs {:.3}", m.to_css(), m.chroma(), c.chroma());
            assert!(contrast(m, canvas) >= MIN_CONTRAST);
        }
        // Hue is still there.
        let [r, g, b, _] = map_text(hex("#006400"), canvas).to_u8();
        assert!(g > r + 30 && g > b + 30);
    }

    #[test]
    fn mapping_twice_is_a_fixed_point_for_random_colours() {
        // Written out as CSS and read back, a mapped colour maps to itself:
        // backgrounds, borders, and text on the written-out mapped background.
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let again = |c: Rgba| parse(&c.to_css()).unwrap();
        for _ in 0..20_000 {
            let (x, y, a) = (next() & 0xff_ffff, next() & 0xff_ffff, next() % 11);
            let bg = Rgba::from_hex24(x as u32);
            let fg = Rgba::from_hex24(y as u32);
            let tint = Rgba { a: a as f64 / 10.0, ..bg };
            for c in [bg, tint] {
                let m = again(map_background(c));
                assert_eq!(map_background(m).to_u8(), m.to_u8(), "bg {}", c.to_css());
                let b = again(map_border(c));
                assert_eq!(map_border(b).to_u8(), b.to_u8(), "border {}", c.to_css());
            }
            let m = again(map_background(bg));
            let t = again(map_text(fg, m));
            assert_eq!(map_text(t, m).to_u8(), t.to_u8(), "text {} on {}", fg.to_css(), bg.to_css());
            assert!(contrast(t, m) >= MIN_CONTRAST, "text {} on {}", fg.to_css(), bg.to_css());
        }
        // The reviewers' cases.
        let m = again(map_background(hex("#127829")));
        assert_eq!(again(map_background(m)), m);
        for (fg, bg) in [("#0afc1a", "#625a86"), ("#badb02", "#625a86")] {
            let t = again(map_text(hex(fg), hex(bg)));
            assert_eq!(again(map_text(t, hex(bg))), t, "{fg}");
        }
    }
}
