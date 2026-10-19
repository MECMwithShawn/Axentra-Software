//! Small text helpers shared by every module.

/// Up to two uppercase initials: "Megan Bowen" -> "MB", "Contoso" -> "C",
/// "info@contoso.com" -> "I". Ignores bracketed suffixes like "(External)".
pub fn initials(name: &str) -> String {
    let base = name.split(['(', '<', '@']).next().unwrap_or(name);
    let words: Vec<&str> = base
        .split(|c: char| c.is_whitespace() || c == ',' || c == '.' || c == '_' || c == '-')
        .filter(|w| w.chars().next().is_some_and(|c| c.is_alphanumeric()))
        .collect();
    let mut out = String::new();
    match words.as_slice() {
        [] => {}
        [one] => out.extend(one.chars().next().map(|c| c.to_uppercase()).into_iter().flatten()),
        [first, .., last] => {
            for w in [first, last] {
                out.extend(w.chars().next().map(|c| c.to_uppercase()).into_iter().flatten());
            }
        }
    }
    out
}

/// Collapse whitespace and cut to `max` characters, adding nothing.
pub fn preview(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(max).collect()
}

/// Deterministic avatar colour index (0..PERSONA_COLORS.len()) for a name or address.
pub fn persona_color_index(key: &str) -> usize {
    // FNV-1a, stable across runs and platforms.
    let mut h: u32 = 0x811c9dc5;
    for b in key.trim().to_lowercase().bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    (h as usize) % PERSONA_COLORS.len()
}

/// Avatar background colours (light theme bg, dark theme bg), Fluent persona palette.
pub const PERSONA_COLORS: &[(&str, &str)] = &[
    ("#c4314b", "#d6576c"),
    ("#ca5010", "#e3782f"),
    ("#986f0b", "#c29a33"),
    ("#498205", "#6ea729"),
    ("#038387", "#2aa5a8"),
    ("#0078d4", "#3a9bea"),
    ("#4f6bed", "#7389f2"),
    ("#5c2e91", "#8758bd"),
    ("#881798", "#ad44bd"),
    ("#e3008c", "#ef4ba9"),
    ("#8e562e", "#b07a52"),
    ("#69797e", "#8fa0a5"),
    ("#0b6a0b", "#2f962f"),
    ("#004e8c", "#2f7bba"),
    ("#a4262c", "#c75157"),
    ("#7a7574", "#a19f9d"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_cover_common_shapes() {
        assert_eq!(initials("Megan Bowen"), "MB");
        assert_eq!(initials("Contoso"), "C");
        assert_eq!(initials("info@contoso.com"), "I");
        assert_eq!(initials("Bowen, Megan (External)"), "BM");
        assert_eq!(initials("  "), "");
        assert_eq!(initials("Ana María de la Cruz"), "AC");
    }

    #[test]
    fn preview_collapses_whitespace() {
        assert_eq!(preview("a\n\n  b\tc", 10), "a b c");
        assert_eq!(preview("abcdef", 3), "abc");
    }
}
