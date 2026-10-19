//! People avatars: Fluent's deterministic initials and colour rules
//! (docs/spec/people-todo.md 2.6), photos, and square list avatars.

use axentra_core::{Contact, ContactList};
use dioxus::prelude::*;

use super::text::nfc;
use crate::kit::Icon;

/// Fluent `getHashCode`: iterates UTF-16 code units, like JS `charCodeAt`.
pub fn fluent_hash(s: &str) -> u32 {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut h: u32 = 0;
    for i in (0..units.len()).rev() {
        let ch = units[i] as u32;
        let shift = (i % 8) as u32;
        h ^= (ch << shift).wrapping_add(ch >> (8 - shift));
    }
    h
}

/// Index into the 30-colour palette, or `None` for the neutral colour.
pub fn color_index(key: &str) -> Option<usize> {
    let key = nfc(key);
    (!key.is_empty()).then(|| (fluent_hash(&key) % 30) as usize)
}

/// Colour key of a contact: primary address (trimmed, lower-case), else the
/// display name.
pub fn contact_color_key(c: &Contact) -> String {
    match c.primary_email() {
        Some(e) if !e.trim().is_empty() => e.trim().to_lowercase(),
        _ => c.display_name.clone(),
    }
}

/// Lists are keyed by their object id.
pub fn list_color_key(l: &ContactList) -> String {
    format!("list:{}", l.id)
}

fn is_unwanted(c: char) -> bool {
    let u = c as u32;
    u < 0x20
        || u == 0x7f
        || (0x21..=0x2f).contains(&u)
        || (0x3a..=0x40).contains(&u)
        || (0x5b..=0x60).contains(&u)
        || (0x7b..=0xbf).contains(&u)
        || (0x250..=0x36f).contains(&u)
        || (0xe000..=0xffff).contains(&u)
}

fn is_no_initials_script(c: char) -> bool {
    let u = c as u32;
    [
        (0x0600, 0x06ff), // Arabic
        (0x0750, 0x077f),
        (0x08a0, 0x08ff),
        (0x1100, 0x11ff), // Hangul Jamo
        (0x3130, 0x318f),
        (0xa960, 0xa97f),
        (0xac00, 0xd7ff), // Hangul syllables
        (0x3040, 0x309f), // Hiragana
        (0x30a0, 0x30ff), // Katakana
        (0x3400, 0x4dbf), // CJK (BMP)
        (0x4e00, 0x9fff),
        (0xf900, 0xfaff),
    ]
    .iter()
    .any(|(a, b)| (*a..=*b).contains(&u))
}

/// Digits and spaces, optionally followed by "ext"/"x" and more digits.
fn looks_like_phone(s: &str) -> bool {
    let lower = s.to_lowercase();
    let (main, ext) = match lower.find("ext").or_else(|| lower.find('x')) {
        Some(i) => (&lower[..i], Some(lower[i..].trim_start_matches("ext").trim_start_matches('x'))),
        None => (lower.as_str(), None),
    };
    let main_ok = main.chars().any(|c| c.is_ascii_digit()) && main.chars().all(|c| c.is_ascii_digit() || c == ' ');
    let ext_ok = ext.is_none_or(|e| {
        let e = e.trim();
        !e.is_empty() && e.chars().all(|c| c.is_ascii_digit())
    });
    main_ok && ext_ok
}

/// Fluent `getInitials`, ported per 2.6.2: `None` means "show the person icon".
pub fn initials(name: &str) -> Option<String> {
    // Drop bracketed enclosures; an unmatched opener drops the rest.
    let mut cleaned = String::with_capacity(name.len());
    let mut in_bracket = false;
    for c in name.chars() {
        match c {
            '(' | '[' | '{' => in_bracket = true,
            ')' | ']' | '}' if in_bracket => in_bracket = false,
            _ if in_bracket => {}
            _ if is_unwanted(c) => {}
            _ => cleaned.push(c),
        }
    }
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let first = collapsed.chars().next()?;
    if is_no_initials_script(first) || looks_like_phone(&collapsed) {
        return None;
    }
    let words: Vec<&str> = collapsed.split(' ').collect();
    let initial = |w: &str| w.chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    let mut out = initial(words[0]);
    match words.len() {
        2 => out.push_str(&initial(words[1])),
        3 => out.push_str(&initial(words[2])),
        _ => {}
    }
    Some(out)
}

/// Initials font size for an avatar size (Fluent size rule).
fn font_px(size: u32) -> u32 {
    match size {
        0..=24 => 10,
        25..=28 => 12,
        29..=40 => 14,
        41..=56 => 16,
        57..=96 => 20,
        _ => 24,
    }
}

/// Person icon size for an avatar size.
fn icon_px(size: u32) -> u32 {
    match size {
        0..=16 => 12,
        17..=24 => 16,
        25..=40 => 20,
        41..=48 => 24,
        49..=56 => 28,
        57..=72 => 32,
        _ => 48,
    }
}

/// Corner radius of square (list/group) avatars.
fn square_radius(size: u32) -> u32 {
    match size {
        0..=24 => 2,
        25..=48 => 4,
        49..=72 => 6,
        _ => 8,
    }
}

/// A person or list avatar. `name` gives the initials, `color_key` the colour;
/// a `photo` (data URL) wins over both.
#[component]
pub fn PeopleAvatar(
    #[props(into)] name: String,
    #[props(into)] color_key: String,
    #[props(default = 32)] size: u32,
    #[props(default)] square: bool,
    #[props(default)] photo: Option<String>,
    #[props(into, default = "person".to_string())] fallback_icon: String,
) -> Element {
    let shape = if square { format!("border-radius:{}px", square_radius(size)) } else { String::new() };
    let base = format!("width:{size}px;height:{size}px;{shape}");
    if let Some(src) = photo.filter(|p| p.starts_with("data:image/")) {
        return rsx! {
            span { class: "ax-people-avatar ax-people-avatar-photo", style: "{base}", "aria-hidden": "true",
                img { src: "{src}", alt: "" }
            }
        };
    }
    let text = initials(&name).map(|i| if size <= 16 { i.chars().take(1).collect() } else { i });
    let color = text.as_ref().and_then(|_| color_index(&color_key));
    rsx! {
        match (text, color) {
            (Some(t), Some(idx)) => rsx! {
                span {
                    class: "ax-people-avatar ax-people-av-{idx}",
                    style: "{base};font-size:{font_px(size)}px",
                    "aria-hidden": "true",
                    "{t}"
                }
            },
            _ => rsx! {
                span { class: "ax-people-avatar ax-people-avatar-neutral", style: "{base}", "aria-hidden": "true",
                    Icon { name: fallback_icon.clone(), size: icon_px(size) }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_vectors_from_the_spec() {
        for (key, hash, index) in [
            ("", 0, 0),
            ("Ada Lovelace", 9219, 9),
            ("ada.lovelace@example.com", 8611, 1),
            ("Grace Hopper", 10076, 26),
            ("Alan Turing", 12203, 23),
            ("José Núñez", 19210, 10),
            ("李雷", 84142, 22),
            ("Jose\u{301} Nu\u{301}n\u{303}ez", 5203, 13),
        ] {
            assert_eq!(fluent_hash(key), hash, "{key}");
            assert_eq!(fluent_hash(key) % 30, index);
        }
        // NFD keys are normalised first, so they get the NFC colour.
        assert_eq!(color_index("Jose\u{301} Nu\u{301}n\u{303}ez"), Some(10));
        assert_eq!(color_index(""), None);
    }

    #[test]
    fn initials_examples_from_the_spec() {
        assert_eq!(initials("Ada Lovelace").as_deref(), Some("AL"));
        assert_eq!(initials("Mary Ann Smith").as_deref(), Some("MS"));
        assert_eq!(initials("Juan Carlos de la Vega").as_deref(), Some("J"));
        assert_eq!(initials("+1 555 0100"), None);
        assert_eq!(initials("555 0100 ext 12"), None);
        assert_eq!(initials("ada@example.com").as_deref(), Some("A"));
        assert_eq!(initials("Ada Lovelace (Contoso)").as_deref(), Some("AL"));
        assert_eq!(initials("Ada [ext"), Some("A".into()));
        assert_eq!(initials("Jean-Luc Picard").as_deref(), Some("JP"));
        assert_eq!(initials("李雷"), None);
        assert_eq!(initials("김민수"), None);
        assert_eq!(initials("  "), None);
        assert_eq!(initials("émile dubois").as_deref(), Some("ÉD"));
        assert_eq!(initials("Box 12x").as_deref(), Some("B1"));
    }

    #[test]
    fn size_rules() {
        assert_eq!((font_px(24), font_px(32), font_px(72), font_px(96), font_px(120)), (10, 14, 20, 20, 24));
        assert_eq!((icon_px(32), icon_px(72), icon_px(96)), (20, 32, 48));
        assert_eq!((square_radius(24), square_radius(32), square_radius(96)), (2, 4, 8));
    }
}
