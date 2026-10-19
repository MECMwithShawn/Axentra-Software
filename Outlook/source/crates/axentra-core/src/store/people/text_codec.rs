//! Byte-level helpers for contact import files: charset detection
//! (UTF-8 / UTF-16 BOMs, Windows-1252 fallback) and base64.

/// Decode an import file. Honours UTF-8 and UTF-16 byte-order marks, then
/// tries UTF-8; if that fails, falls back to Windows-1252 and returns `true`
/// so the UI can warn "Some characters may not display correctly"
/// (docs/spec/people-todo.md 2.13).
pub(crate) fn decode_bytes(bytes: &[u8]) -> (String, bool) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(rest).into_owned(), false);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return (utf16(rest, u16::from_le_bytes), false);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return (utf16(rest, u16::from_be_bytes), false);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), false),
        Err(_) => (bytes.iter().map(|&b| cp1252_char(b)).collect(), true),
    }
}

fn utf16(bytes: &[u8], read: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| read(*c)).collect();
    String::from_utf16_lossy(&units)
}

/// One Windows-1252 byte as a char (bytes outside 0x80..=0x9F are Latin-1).
pub(crate) fn cp1252_char(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}',
        '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

#[cfg(test)]
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[cfg(test)]
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Standard or URL-safe alphabet; whitespace ignored; padding optional.
pub(crate) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for ch in s.bytes() {
        let v = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits & 0xFF) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_bom_utf8_and_cp1252() {
        assert_eq!(decode_bytes(b"\xEF\xBB\xBFJos\xC3\xA9"), ("José".to_string(), false));
        assert_eq!(decode_bytes("Núñez".as_bytes()), ("Núñez".to_string(), false));
        assert_eq!(decode_bytes(b"Jos\xE9 \x93hi\x94 \x80"), ("José “hi” €".to_string(), true));
        assert_eq!(decode_bytes(b"\xFF\xFEA\x00\xE9\x00"), ("Aé".to_string(), false));
        assert_eq!(decode_bytes(b"\xFE\xFF\x00A\x00\xE9"), ("Aé".to_string(), false));
    }

    #[test]
    fn base64_round_trip() {
        for input in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7]] {
            let enc = base64_encode(input);
            assert_eq!(base64_decode(&enc).unwrap(), input);
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_decode("Zm9v\nYmE=").unwrap(), b"fooba");
        assert!(base64_decode("not*base64").is_none());
    }
}
