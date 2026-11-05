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

/// MIME type from a file name, for files the platform did not label.
pub fn guess_content_type(name: &str) -> String {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "ics" => "text/calendar",
        "eml" => "message/rfc822",
        "json" => "application/json",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Picture type from a file's first bytes, whatever it is called.
pub fn sniff_image_type(data: &[u8]) -> Option<&'static str> {
    Some(match data {
        [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', ..] => "image/png",
        [0xff, 0xd8, 0xff, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [b'B', b'M', ..] if data.len() > 14 => "image/bmp",
        _ => return None,
    })
}

/// The type to give an attached file: what its bytes show for a picture,
/// then a specific type from its name, then the platform's label. The file
/// picker labels some files wrongly (a PNG as text, or as unknown bytes).
pub fn attachment_content_type(name: &str, given: &str, data: &[u8]) -> String {
    if let Some(t) = sniff_image_type(data) {
        return t.to_string();
    }
    // A picture type these checks recognise, named but not in the bytes:
    // not that picture after all.
    let sniffable = |t: &str| matches!(t, "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp");
    let guessed = guess_content_type(name);
    if guessed != "application/octet-stream" && !sniffable(&guessed) {
        return guessed;
    }
    let given = given.trim().to_ascii_lowercase();
    let generic = given.is_empty() || given == "application/octet-stream" || given == "binary/octet-stream";
    if !generic && !sniffable(&given) {
        return given;
    }
    "application/octet-stream".to_string()
}

/// True when the bytes read as text: no NUL and valid UTF-8 in the first
/// 8 KiB (a cut at the end is allowed).
pub fn looks_like_text(data: &[u8]) -> bool {
    let head = &data[..data.len().min(8192)];
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && head.len() == 8192,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attached_files_get_their_real_type() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        let jpeg = b"\xff\xd8\xff\xe0\0\x10JFIF";
        // Labelled wrongly by the file picker: the bytes win.
        assert_eq!(attachment_content_type("IMG_2411.PNG", "text/plain", png), "image/png");
        assert_eq!(attachment_content_type("IMG_2415.jpeg", "application/octet-stream", jpeg), "image/jpeg");
        assert_eq!(attachment_content_type("photo", "", png), "image/png");
        // Called a picture but isn't one.
        assert_eq!(attachment_content_type("fake.png", "image/png", b"hello"), "application/octet-stream");
        // Otherwise the name, then the label.
        assert_eq!(attachment_content_type("Plan.pdf", "", b"%PDF-1.7"), "application/pdf");
        assert_eq!(attachment_content_type("notes", "text/plain", b"hi"), "text/plain");
        assert_eq!(attachment_content_type("data.bin", "", b"\0\x01"), "application/octet-stream");
        assert_eq!(attachment_content_type("logo.svg", "", b"<svg/>"), "image/svg+xml");
        assert_eq!(attachment_content_type("IMG_1.HEIC", "image/heic", b"\0\0\0\x18ftypheic"), "image/heic");
        assert!(looks_like_text("Grüße\n".as_bytes()));
        assert!(!looks_like_text(png));
        assert!(!looks_like_text(b"\xff\xfe\xfd"));
    }

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
