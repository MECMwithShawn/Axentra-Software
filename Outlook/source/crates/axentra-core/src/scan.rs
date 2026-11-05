//! Capped, UTF-8-safe text preparation shared by the junk checks, order
//! detection and topic sorting. HTML is scanned as text (never rendered, no
//! resources loaded); links are read but never followed. Every cut lands on
//! a character boundary, and every scan is linear.

use std::borrow::Cow;

use crate::model::Message;

/// Links read from one message at most.
pub(crate) const MAX_LINKS: usize = 200;
/// Text after an unclosed `<a …>` read as its link text.
pub(crate) const MAX_LINK_TEXT: usize = 2000;

const MAX_HTML: usize = 512 * 1024;
/// Scanned body text (after the HTML is turned into text).
const MAX_BODY: usize = 96 * 1024;
const MAX_SUBJECT: usize = 2048;
/// The part of the body read as its "head" (status phrases, labels).
const MAX_HEAD: usize = 4 * 1024;

/// The body sync stores in place of a message too large to download.
const TOO_LARGE: &str = "This message is too large to download";

/// The largest index `<= max` that is a character boundary of `s` (a byte
/// index, not a slice).
pub(crate) fn floor_char(s: &str, max: usize) -> usize {
    if s.len() <= max {
        return s.len();
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// `(href, text)` pairs of `<a>` tags, scanned as text. Returns true when
/// the link limit was hit.
pub(crate) fn extract_links(html: &str, out: &mut Vec<(String, String)>) -> bool {
    let lower = html.to_ascii_lowercase();
    let mut pos = 0;
    // The next "</a" at or after `pos`, searched once and reused, so many
    // unclosed anchors stay linear instead of rescanning to the end each time.
    let mut next_close: Option<Option<usize>> = None;
    while let Some(start) = lower[pos..].find("<a").map(|i| pos + i) {
        pos = start + 2;
        if !lower[pos..].starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        let Some(tag_end) = lower[pos..].find('>').map(|i| pos + i) else { break };
        let tag = &html[pos..tag_end];
        let href = attr(tag, "href");
        if next_close.is_some_and(|c| c.is_some_and(|c| c < tag_end)) {
            next_close = None;
        }
        let found = *next_close.get_or_insert_with(|| lower[tag_end..].find("</a").map(|i| tag_end + i));
        // Unclosed: the text up to a cap, ending on a character boundary.
        let close = found.unwrap_or_else(|| tag_end + 1 + floor_char(&html[tag_end + 1..], MAX_LINK_TEXT));
        let inner = strip_tags(&html[tag_end + 1..close.max(tag_end + 1)]);
        pos = close.max(tag_end + 1);
        if let Some(href) = href {
            if out.len() >= MAX_LINKS {
                return true;
            }
            out.push((href, inner));
        }
    }
    false
}

pub(crate) fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name).map(|i| from + i) {
        from = i + name.len();
        let before_ok = i == 0 || lower.as_bytes()[i - 1].is_ascii_whitespace();
        let rest = lower[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let vstart = tag.len() - rest.len() + 1;
        let v = tag[vstart..].trim_start();
        let value = match v.chars().next() {
            Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or(""),
            _ => v.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or(""),
        };
        return Some(decode_entities(value.trim()));
    }
    None
}

pub(crate) fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(out.split_whitespace().collect::<Vec<_>>().join(" ").trim())
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&").replace("&#46;", ".").replace("&#x2e;", ".").replace("&#64;", "@").replace("&nbsp;", " ").replace("&quot;", "\"")
}

/// HTML as text that keeps its structure: a newline per block, `" | "`
/// between table cells, entities decoded, at most `cap` bytes. Returns the
/// text and whether it was cut. (Separate from `html::html_to_text`, which
/// feeds search and the preview line.)
pub(crate) fn structured_text(html: &str, cap: usize) -> (String, bool) {
    let lower = html.to_ascii_lowercase();
    let mut out = Text { s: String::new(), cap, cut: false, space: false, cell: false };
    let mut pos = 0;
    while pos < html.len() && !out.cut {
        let Some(lt) = html[pos..].find(['<', '&']).map(|i| pos + i) else {
            out.text(&html[pos..]);
            break;
        };
        out.text(&html[pos..lt]);
        if html.as_bytes()[lt] == b'&' {
            let (decoded, used) = entity(&html[lt..]);
            match decoded {
                Some(c) => out.text(c.encode_utf8(&mut [0; 4])),
                None => out.text("&"),
            }
            pos = lt + used;
            continue;
        }
        let rest = &lower[lt + 1..];
        if rest.starts_with("!--") {
            pos = rest.find("-->").map_or(html.len(), |i| lt + 1 + i + 3);
            continue;
        }
        // Only `<` followed by a letter, `/` or `!` opens a tag; anything
        // else is text ("a < b").
        if !rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!') {
            out.text("<");
            pos = lt + 1;
            continue;
        }
        let Some(gt) = rest.find('>').map(|i| lt + 1 + i) else {
            // An unclosed tag runs to the end: nothing more to read.
            break;
        };
        let closing = rest.starts_with('/');
        let name: String = rest[usize::from(closing)..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        pos = gt + 1;
        if !closing && matches!(name.as_str(), "style" | "script" | "head" | "title") {
            let end = format!("</{name}");
            pos = lower[pos..].find(&end).map_or(html.len(), |i| pos + i);
            continue;
        }
        match name.as_str() {
            "td" | "th" if !closing => {
                if out.cell {
                    out.raw(" | ");
                }
                out.cell = true;
            }
            "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "table" | "section" | "article" | "header" | "footer" => {
                out.newline();
            }
            _ => {}
        }
    }
    let cut = out.cut;
    let s = out.s.trim_end().to_string();
    (s, cut)
}

/// The output of [`structured_text`]: spaces collapsed, at most `cap` bytes.
struct Text {
    s: String,
    cap: usize,
    cut: bool,
    /// A space is owed before the next visible character.
    space: bool,
    /// A table cell has started on this row.
    cell: bool,
}

impl Text {
    fn text(&mut self, t: &str) {
        for c in t.chars() {
            if c.is_whitespace() {
                self.space = true;
            } else {
                if self.space && !self.s.is_empty() && !self.s.ends_with('\n') && !self.s.ends_with(' ') {
                    self.push(' ');
                }
                self.space = false;
                self.push(c);
            }
            if self.cut {
                return;
            }
        }
    }

    fn raw(&mut self, t: &str) {
        self.space = false;
        while self.s.ends_with(' ') {
            self.s.pop();
        }
        for c in t.chars() {
            self.push(c);
        }
    }

    fn newline(&mut self) {
        self.space = false;
        self.cell = false;
        while self.s.ends_with(' ') {
            self.s.pop();
        }
        if !self.s.is_empty() && !self.s.ends_with("\n") {
            self.push('\n');
        }
    }

    fn push(&mut self, c: char) {
        if self.s.len() + c.len_utf8() > self.cap {
            self.cut = true;
        } else {
            self.s.push(c);
        }
    }
}

/// The entity at the start of `s` (which starts with `&`) and the bytes it
/// takes; `(None, 1)` when it isn't one.
fn entity(s: &str) -> (Option<char>, usize) {
    let body = &s[1..floor_char(s, 12)];
    let Some(semi) = body.find(';') else { return (None, 1) };
    let name = &body[..semi];
    let used = semi + 2;
    let c = if let Some(num) = name.strip_prefix('#') {
        let n = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => num.parse::<u32>().ok(),
        };
        let Some(n) = n else { return (None, 1) };
        char::from_u32(n).filter(|c| *c != '\0').unwrap_or('\u{FFFD}')
    } else {
        match name {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            "nbsp" => ' ',
            "euro" => '€',
            "pound" => '£',
            "yen" => '¥',
            "cent" => '¢',
            "rsquo" => '\u{2019}',
            "lsquo" => '\u{2018}',
            "ldquo" => '\u{201c}',
            "rdquo" => '\u{201d}',
            "ndash" => '\u{2013}',
            "mdash" => '\u{2014}',
            "hellip" => '\u{2026}',
            "times" => '×',
            "middot" => '·',
            "copy" => '©',
            "reg" => '®',
            "trade" => '™',
            _ => return (None, 1),
        }
    };
    (Some(c), used)
}

/// Full-width ASCII forms (U+FF01–U+FF5E) as ASCII, and the ideographic
/// space (U+3000) as a space.
pub(crate) fn normalize_fullwidth(s: &str) -> Cow<'_, str> {
    if !s.chars().any(|c| matches!(c, '\u{FF01}'..='\u{FF5E}' | '\u{3000}')) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(
        s.chars()
            .map(|c| match c {
                '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFF01 + 0x21).unwrap_or(c),
                '\u{3000}' => ' ',
                c => c,
            })
            .collect(),
    )
}

/// One message, capped and ready to scan.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Prepared {
    pub subject: String,
    /// Subject, a newline, then the first 4 KiB of the body.
    pub head: String,
    pub body: String,
    /// `head` and `body` ASCII-lowercased (same byte offsets).
    pub head_lower: String,
    pub body_lower: String,
    /// `(href, text)` of the links in the (capped) HTML.
    pub links: Vec<(String, String)>,
    pub truncated: bool,
    /// The body is the "too large to download" placeholder (subject only).
    pub too_large: bool,
}

pub(crate) fn prepare(message: &Message, html: Option<&str>, text: &str) -> Prepared {
    let subject = normalize_fullwidth(&message.subject[..floor_char(&message.subject, MAX_SUBJECT)]).into_owned();
    let mut out = Prepared { truncated: subject.len() < message.subject.len(), ..Default::default() };
    if text.starts_with(TOO_LARGE) {
        out.too_large = true;
    } else if let Some(full) = html {
        let html = &full[..floor_char(full, MAX_HTML)];
        out.truncated |= html.len() < full.len();
        out.truncated |= extract_links(html, &mut out.links);
        let (body, cut) = structured_text(html, MAX_BODY);
        out.truncated |= cut;
        out.body = normalize_fullwidth(&body).into_owned();
    } else {
        let cut = floor_char(text, MAX_BODY);
        out.truncated |= cut < text.len();
        out.body = normalize_fullwidth(&text[..cut]).into_owned();
    }
    out.head = format!("{subject}\n{}", &out.body[..floor_char(&out.body, MAX_HEAD)]);
    out.head_lower = out.head.to_ascii_lowercase();
    out.body_lower = out.body.to_ascii_lowercase();
    out.subject = subject;
    out
}

/// Byte offsets of `needle` (ASCII lowercase, not empty) in `hay_lower`.
pub(crate) fn find_ascii<'a>(hay_lower: &'a str, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    hay_lower.match_indices(needle).map(|(i, _)| i)
}

/// True when byte `i` of `hay` is a word boundary: the characters on either
/// side are not both letters or digits.
pub(crate) fn word_at(hay: &str, i: usize) -> bool {
    if !hay.is_char_boundary(i) {
        return false;
    }
    let before = hay[..i].chars().next_back().is_some_and(char::is_alphanumeric);
    let after = hay[i..].chars().next().is_some_and(char::is_alphanumeric);
    !(before && after)
}

/// Offsets of `needle` in `hay_lower` as a whole word. The boundary is only
/// checked at an end of `needle` that is an ASCII letter or digit, so
/// "delivered:" and CJK phrases match inside running text.
pub(crate) fn find_word<'a>(hay_lower: &'a str, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    let first = needle.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric());
    let last = needle.bytes().next_back().is_some_and(|b| b.is_ascii_alphanumeric());
    find_ascii(hay_lower, needle).filter(move |&i| (!first || word_at(hay_lower, i)) && (!last || word_at(hay_lower, i + needle.len())))
}

pub(crate) fn contains_word(hay_lower: &str, needle: &str) -> bool {
    find_word(hay_lower, needle).next().is_some()
}

/// "tel", "call" or "phone" starts a word within 20 bytes before `at`.
pub(crate) fn phone_word_before(lower: &str, at: usize) -> bool {
    let window = &lower[floor_char(lower, at.saturating_sub(20))..floor_char(lower, at)];
    ["tel", "call", "phone"].iter().any(|w| find_ascii(window, w).any(|i| word_at(window, i)))
}

/// True when the local part of an address (lowercase) is `word`, or has it
/// as a `.`, `_`, `+` or `-` separated part ("orders", "eu.orders").
pub(crate) fn local_part_is(local: &str, word: &str) -> bool {
    local == word || local.split(['.', '_', '+', '-']).any(|p| p == word) || (word.contains('-') && local.starts_with(word))
}

/// Words in a link's text or address that make it an unsubscribe link.
const UNSUBSCRIBE: &[&str] = &["unsubscribe", "abmelden", "désinscrire", "desinscrire", "darse de baja", "配信停止"];

/// The message has an unsubscribe link.
pub(crate) fn has_unsubscribe(p: &Prepared) -> bool {
    p.links.iter().any(|(href, text)| {
        let (href, text) = (href.to_ascii_lowercase(), text.to_ascii_lowercase());
        UNSUBSCRIBE.iter().any(|w| href.contains(w) || text.contains(w))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_width_and_boundaries() {
        assert_eq!(normalize_fullwidth("ＳＺ－１００２４５"), "SZ-100245");
        assert!(matches!(normalize_fullwidth("plain"), Cow::Borrowed(_)));
        assert_eq!(normalize_fullwidth("a\u{3000}b"), "a b");
        assert!(word_at("ups tracking", 0) && word_at("ups tracking", 3) && !word_at("ups tracking", 1));
        assert!(!word_at("é", 1), "not a character boundary");
        assert_eq!(find_ascii("order #1, order #2", "order").collect::<Vec<_>>(), [0, 10]);
    }

    #[test]
    fn structured_text_keeps_rows_and_cells() {
        let html = "<html><head><title>Receipt</title><style>p { color: red }</style></head><body>\
                    <h1>Your  order</h1><p>Thanks,&nbsp;Alex &amp; co</p><!-- hidden <p>x</p> -->\
                    <table><tr><th>Item</th><th>Qty</th></tr><tr><td> Mug </td><td>2</td><td>&#36;9.00 &#x20AC;8 &euro;</td></tr></table>\
                    <script>var a = '<p>no</p>';</script><div>a &lt; b, 3 < 4 &bogus; &#xFFFFFF;</div></body></html>";
        let (text, cut) = structured_text(html, 10_000);
        assert!(!cut);
        assert_eq!(text, "Your order\nThanks, Alex & co\nItem | Qty\nMug | 2 | $9.00 €8 €\na < b, 3 < 4 &bogus; \u{FFFD}");
    }

    #[test]
    fn structured_text_is_capped_on_a_character_boundary() {
        let (text, cut) = structured_text(&"<p>é€𝄞</p>".repeat(1000), 1001);
        assert!(cut && text.len() <= 1001);
        let (text, cut) = structured_text("<p>unclosed <b", 100);
        assert_eq!((text.as_str(), cut), ("unclosed", false));
        assert_eq!(structured_text(&"<".repeat(50), 100).0, "<".repeat(50));
    }

    #[test]
    fn prepare_reads_html_and_links() {
        let m = Message { subject: "Hi".into(), ..Default::default() };
        let p = prepare(&m, Some("<p>Ａ <a href=\"https://x.example/unsubscribe\">Leave</a></p>"), "ignored");
        assert_eq!(p.body, "A Leave");
        assert_eq!(p.head, "Hi\nA Leave");
        assert_eq!(p.links.len(), 1);
        assert!(has_unsubscribe(&p));
        assert!(local_part_is("eu.orders", "orders") && local_part_is("no-reply", "no-reply") && !local_part_is("borders", "orders"));
        assert!(contains_word("delivered: today", "delivered:") && !contains_word("undelivered", "delivered"));
        assert!(phone_word_before("call 555", 5) && !phone_word_before("hotel 555", 6));
    }

    #[test]
    fn too_large_placeholder_keeps_only_the_subject() {
        let m = Message { subject: "Ｏｒｄｅｒ shipped".into(), ..Default::default() };
        let p = prepare(&m, None, "This message is too large to download. Open it in your provider's web mail.");
        assert!(p.too_large && p.body.is_empty());
        assert_eq!(p.subject, "Order shipped");
        assert_eq!(p.head_lower, "order shipped\n");
    }
}
