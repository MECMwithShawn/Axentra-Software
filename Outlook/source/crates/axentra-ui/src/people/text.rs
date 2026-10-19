//! Text helpers for People: NFC composition of Latin letters (avatar colour
//! keys), accent folding for sorting and search, index headings.
//!
//! There is no ICU in the dependency tree, so collation is an approximation:
//! accents fold to their base letter (so "Á" files under "A"), case is
//! ignored, and digits/symbols file under "#" after every letter.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::compose_table::COMPOSE;

fn compose_pair(base: char, mark: char) -> Option<char> {
    COMPOSE.binary_search_by(|(b, m, _)| (*b, *m).cmp(&(base, mark))).ok().map(|i| COMPOSE[i].2)
}

fn decompose_map() -> &'static HashMap<char, char> {
    static MAP: OnceLock<HashMap<char, char>> = OnceLock::new();
    MAP.get_or_init(|| COMPOSE.iter().map(|(b, _, c)| (*c, *b)).collect())
}

fn is_combining(c: char) -> bool {
    ('\u{300}'..='\u{36f}').contains(&c)
}

/// Canonical composition (NFC) for Latin letters followed by combining
/// marks. Other text passes through unchanged.
pub fn nfc(s: &str) -> String {
    let mut out: Vec<char> = Vec::with_capacity(s.len());
    for c in s.chars() {
        if is_combining(c)
            && let Some(last) = out.last_mut()
            && let Some(composed) = compose_pair(*last, c)
        {
            *last = composed;
            continue;
        }
        out.push(c);
    }
    out.into_iter().collect()
}

/// Base letter of an accented Latin letter (recursively), plus a few letters
/// that have no canonical decomposition.
fn base_char(c: char) -> char {
    let mut c = c;
    while let Some(b) = decompose_map().get(&c) {
        c = *b;
    }
    match c {
        'ø' => 'o',
        'Ø' => 'O',
        'ł' => 'l',
        'Ł' => 'L',
        'đ' => 'd',
        'Đ' => 'D',
        'ħ' => 'h',
        'Ħ' => 'H',
        'ı' => 'i',
        other => other,
    }
}

/// Lower-case, accent-free form used for sorting and matching.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_combining(c) {
            continue;
        }
        match c {
            'ß' => out.push_str("ss"),
            'æ' | 'Æ' => out.push_str("ae"),
            'œ' | 'Œ' => out.push_str("oe"),
            other => out.extend(base_char(other).to_lowercase()),
        }
    }
    out
}

/// Index heading for a sort key: the upper-case base letter of its first
/// character; the character itself for scripts without case; "#" for
/// digits, symbols and empty keys.
pub fn index_heading(key: &str) -> String {
    match key.trim().chars().next() {
        Some(c) if c.is_alphabetic() => {
            let b = base_char(c);
            match b {
                'ß' => "S".into(),
                'æ' | 'Æ' => "A".into(),
                'œ' | 'Œ' => "O".into(),
                _ if b.is_lowercase() || b.is_uppercase() => b.to_uppercase().collect(),
                _ => b.to_string(),
            }
        }
        _ => "#".into(),
    }
}

/// Only the ASCII digits of `s` (phone matching ignores formatting).
pub fn phone_digits(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_nfd_latin() {
        assert_eq!(nfc("Jose\u{301} Nu\u{301}n\u{303}ez"), "José Núñez");
        assert_eq!(nfc("e\u{302}\u{301}"), "ế");
        assert_eq!(nfc("李雷"), "李雷");
        assert_eq!(nfc("\u{301}x"), "\u{301}x");
    }

    #[test]
    fn folds_accents_and_case() {
        assert_eq!(fold("Émile Ångström"), "emile angstrom");
        assert_eq!(fold("Straße Øre Łódź"), "strasse ore lodz");
        assert_eq!(fold("Jose\u{301}"), "jose");
    }

    #[test]
    fn headings() {
        assert_eq!(index_heading("Ádám"), "A");
        assert_eq!(index_heading("émile"), "E");
        assert_eq!(index_heading("Øre"), "O");
        assert_eq!(index_heading("42 Club"), "#");
        assert_eq!(index_heading("+1 555"), "#");
        assert_eq!(index_heading(""), "#");
        assert_eq!(index_heading("李雷"), "李");
        assert_eq!(index_heading("ж"), "Ж");
    }
}
