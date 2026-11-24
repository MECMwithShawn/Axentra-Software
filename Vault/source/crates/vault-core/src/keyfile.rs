//! KeePass key files. [`effective_key`] is the key-file part of the composite key exactly as
//! keepass 0.15.2 derives it (`key::parse_keyfile`), and [`raw_key`] is the 32-byte key file that
//! reproduces it: what a passkey wraps. The backup `.keyx` written at passkey setup is a KeePass
//! 2.x XML key file (format 2.0) holding those 32 bytes, so it opens the database in any
//! KeePass-compatible app.

use base64::Engine as _;
use quick_xml::{XmlVersion, events::Event, reader::Reader};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const EXTENSION: &str = "keyx";

/// The suggested name for a new backup key file: `{stem}-backup YYYY-MM-DD.keyx`, dated today
/// (local time). Backup key files are never overwritten, so each key change needs a new name.
pub fn backup_file_name(stem: &str) -> String {
    format!(
        "{stem}-backup {}.{EXTENSION}",
        chrono::Local::now().format("%Y-%m-%d")
    )
}

/// The key-file element keepass 0.15.2 puts into the composite key for these file contents, in
/// its order:
/// 1. an XML key file's `KeyFile/Key/Data` text: hex for `<Version>2.0</Version>`, otherwise
///    base64, and the text itself when that doesn't decode;
/// 2. a file of exactly 32 bytes, as is;
/// 3. a file of exactly 64 hex characters (nothing trimmed), decoded;
/// 4. otherwise the SHA-256 of the whole file.
///
/// Only an XML key file can give something other than 32 bytes.
pub fn effective_key(data: &[u8]) -> Zeroizing<Vec<u8>> {
    if let Some(element) = xml_element(data) {
        return element;
    }
    if data.len() == 32 {
        return Zeroizing::new(data.to_vec());
    }
    if data.len() == 64
        && let Ok(key) = hex::decode(data)
    {
        return Zeroizing::new(key);
    }
    Zeroizing::new(Sha256::digest(data).to_vec())
}

/// The 32 bytes that, saved as a key file, open whatever `data` opens: its effective key, when
/// that is 32 bytes and KeePass reads a 32-byte file of it back unchanged (one that parses as an
/// XML key file would not be). Every key file qualifies except an XML key file whose data isn't
/// 32 bytes.
pub fn raw_key(data: &[u8]) -> Option<[u8; 32]> {
    let key: [u8; 32] = effective_key(data).as_slice().try_into().ok()?;
    (effective_key(&key)[..] == key[..]).then_some(key)
}

/// keepass's `parse_xml_keyfile`, step for step on the same quick-xml, base64 and hex versions:
/// None wherever keepass gives up on XML and reads the file as bytes instead.
fn xml_element(xml: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    let mut tag_stack: Vec<String> = Vec::new();
    let mut key_version: Option<String> = None;
    let mut key_value: Option<Zeroizing<String>> = None;
    let mut reader = Reader::from_reader(xml);
    // One event never outgrows the file, so no reallocation leaves a copy of the key behind.
    // quick-xml only looks at what each call appends, so clearing between events changes nothing.
    let mut buf = Zeroizing::new(Vec::with_capacity(xml.len()));
    let mut xml_version = XmlVersion::Implicit1_0;
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf).ok()? {
            Event::Eof => break,
            Event::Decl(e) => xml_version = e.xml_version().ok()?,
            Event::Start(e) => tag_stack.push(e.name().as_ref().to_owned()),
            Event::End(_) => {
                tag_stack.pop();
            }
            Event::Text(e) => {
                let s = Zeroizing::new(e.xml_content(xml_version).into_owned());
                if tag_stack == ["KeyFile", "Meta", "Version"] {
                    key_version = Some(s.to_string());
                } else if tag_stack == ["KeyFile", "Key", "Data"] {
                    key_value = Some(s);
                }
            }
            _ => {}
        }
    }
    let key_value = key_value?;
    if key_version.as_deref() == Some("2.0") {
        // keepass trims, then drops spaces, newlines, tabs and carriage returns.
        let mut digits = Zeroizing::new(String::with_capacity(key_value.len()));
        digits.extend(
            key_value
                .trim()
                .chars()
                .filter(|c| !matches!(c, ' ' | '\n' | '\t' | '\r')),
        );
        if let Ok(key) = hex::decode(digits.as_bytes()) {
            return Some(Zeroizing::new(key));
        }
    } else if let Ok(key) = base64::engine::general_purpose::STANDARD.decode(key_value.as_bytes()) {
        return Some(Zeroizing::new(key));
    }
    Some(Zeroizing::new(key_value.as_bytes().to_vec()))
}

pub fn build_v2(key: &[u8; 32]) -> String {
    let hex = hex::encode_upper(key);
    let hash = hex::encode_upper(&Sha256::digest(key)[..4]);
    let groups: Vec<&str> = (0..8).map(|i| &hex[i * 8..i * 8 + 8]).collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n\
         <KeyFile>\r\n\
         \t<Meta>\r\n\t\t<Version>2.0</Version>\r\n\t</Meta>\r\n\
         \t<Key>\r\n\
         \t\t<Data Hash=\"{hash}\">\r\n\
         \t\t\t{}\r\n\
         \t\t\t{}\r\n\
         \t\t</Data>\r\n\
         \t</Key>\r\n\
         </KeyFile>\r\n",
        groups[..4].join(" "),
        groups[4..].join(" "),
    )
}

#[cfg(test)]
mod tests {
    use keepass::{Database, DatabaseKey, config::KdfConfig};

    use super::*;

    const PASSWORD: &str = "keyfile-differential-pw";

    #[test]
    fn backup_file_names_are_dated() {
        let name = backup_file_name("Shawn");
        let date = name
            .strip_prefix("Shawn-backup ")
            .and_then(|n| n.strip_suffix(".keyx"))
            .unwrap();
        assert!(
            chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok(),
            "{name}"
        );
    }

    #[test]
    fn raw_key_from_every_direct_format() {
        let key: [u8; 32] = core::array::from_fn(|i| (i * 7) as u8);
        assert_eq!(raw_key(&key), Some(key));
        assert_eq!(raw_key(hex::encode(key).as_bytes()), Some(key));
        assert_eq!(
            raw_key(build_v2(&key).as_bytes()),
            Some(key),
            "our own .keyx backups"
        );
        // Any other file is hashed by KeePass; its hash is the key a passkey wraps.
        let bare = b"some random file that KeePass would hash";
        assert_eq!(raw_key(bare), Some(Sha256::digest(bare).into()));
    }

    // Same key and expected file as KeePass's own documentation example layout.
    #[test]
    fn format_matches_keepass_layout() {
        let key: [u8; 32] = core::array::from_fn(|i| i as u8);
        let xml = build_v2(&key);
        assert!(xml.contains("<Version>2.0</Version>"));
        assert!(xml.contains("00010203 04050607 08090A0B 0C0D0E0F"));
        assert!(xml.contains("10111213 14151617 18191A1B 1C1D1E1F"));
        let hash = hex::encode_upper(&Sha256::digest(key)[..4]);
        assert!(xml.contains(&format!("Hash=\"{hash}\"")));
    }

    /// An empty database keyed with the master password plus `key_file`, as KDBX bytes.
    fn keyed_with(key_file: &[u8]) -> Vec<u8> {
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
        let mut out = Vec::new();
        db.save(&mut out, key(key_file)).unwrap();
        out
    }

    fn key(key_file: &[u8]) -> DatabaseKey {
        DatabaseKey::new()
            .with_password(PASSWORD)
            .with_keyfile(&mut &key_file[..])
            .unwrap()
    }

    fn opens(kdbx: &[u8], key_file: &[u8]) -> bool {
        Database::parse(kdbx, key(key_file)).is_ok()
    }

    /// A format 1.0 XML key file whose data is `element`, of any length.
    fn v1_xml(element: &[u8]) -> Vec<u8> {
        let data = base64::engine::general_purpose::STANDARD.encode(element);
        format!("<KeyFile><Key><Data>{data}</Data></Key></KeyFile>").into_bytes()
    }

    enum Expect {
        /// The effective key is exactly these bytes, and they work as a raw key file.
        Key([u8; 32]),
        /// KeePass hashes the whole file; the hash works as a raw key file.
        Hashed,
        /// No 32-byte key file reproduces the effective key.
        NotRaw,
    }

    /// Differential check against keepass itself: a database keyed with each file opens with
    /// password + `raw_key` as a 32-byte key file, and with password + `effective_key` in any form.
    #[test]
    fn effective_key_matches_keepass_for_every_form() {
        let k: [u8; 32] = core::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(11));
        let decoy: [u8; 32] = core::array::from_fn(|i| 255 - i as u8);
        let hex_k = hex::encode(k);
        let v2 = |version: &str, data: &str| {
            format!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<KeyFile>\n\t<Meta>\n\t\t<Version>{version}</Version>\n\t</Meta>\n\t<Key>\n\t\t{data}\n\t</Key>\n</KeyFile>\n"
            )
            .into_bytes()
        };
        let spaced: String = hex::encode_upper(k)
            .as_bytes()
            .chunks(6)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join(" \t\r\n   ");
        // 32 bytes of unclosed XML: as a key file, KeePass reads its data ("ABCDEFGHI"), not it.
        let xml_like: [u8; 32] = *b"<KeyFile><Key><Data>QUJDREVGR0hJ";
        let mut bom_hex = b"\xEF\xBB\xBF".to_vec();
        bom_hex.extend_from_slice(hex_k.as_bytes());
        let v1 = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<KeyFile>\r\n\t<Meta>\r\n\t\t<Version>1.00</Version>\r\n\t</Meta>\r\n\t<Key>\r\n\t\t<Data>{}</Data>\r\n\t</Key>\r\n</KeyFile>\r\n",
            base64::engine::general_purpose::STANDARD.encode(k)
        );

        let table: Vec<(&str, Vec<u8>, Expect)> = vec![
            ("raw 32 bytes", k.to_vec(), Expect::Key(k)),
            ("64 hex", hex_k.clone().into_bytes(), Expect::Key(k)),
            (
                "64 HEX upper case",
                hex::encode_upper(k).into_bytes(),
                Expect::Key(k),
            ),
            ("hex + LF", format!("{hex_k}\n").into_bytes(), Expect::Hashed),
            (
                "hex + CRLF",
                format!("{hex_k}\r\n").into_bytes(),
                Expect::Hashed,
            ),
            ("BOM + hex", bom_hex, Expect::Hashed),
            ("v1 XML", v1.into_bytes(), Expect::Key(k)),
            ("build_v2 output", build_v2(&k).into_bytes(), Expect::Key(k)),
            (
                "v2 with extra whitespace",
                v2("2.0", &format!("<Data Hash=\"00000000\">\r\n\n {spaced} \t\r\n</Data>")),
                Expect::Key(k),
            ),
            (
                "wrong root element",
                format!(
                    "<Keyfile><Meta><Version>2.0</Version></Meta><Key><Data>{hex_k}</Data></Key></Keyfile>"
                )
                .into_bytes(),
                Expect::Hashed,
            ),
            (
                "<DataX> decoy before <Data>",
                v2(
                    "2.0",
                    &format!("<DataX>{}</DataX><Data>{hex_k}</Data>", hex::encode(decoy)),
                ),
                Expect::Key(k),
            ),
            (
                "<DataX> decoy only",
                v2("2.0", &format!("<DataX>{hex_k}</DataX>")),
                Expect::Hashed,
            ),
            (
                // Version 1 rules then: 64 hex digits are valid base64 for 48 other bytes.
                "Version only inside a comment",
                format!(
                    "<KeyFile><Meta><!-- <Version>2.0</Version> --></Meta><Key><Data>{hex_k}</Data></Key></KeyFile>"
                )
                .into_bytes(),
                Expect::NotRaw,
            ),
            (
                // Not exactly "2.0", so version 1 rules: the spaced text isn't base64 either.
                "v2 with a padded Version",
                v2(" 2.0 ", &format!("<Data>{spaced}</Data>")),
                Expect::NotRaw,
            ),
            ("32 bytes of XML", xml_like.to_vec(), Expect::NotRaw),
            ("v1 XML holding XML-like bytes", v1_xml(&xml_like), Expect::NotRaw),
        ];

        for (name, file, expect) in table {
            let kdbx = keyed_with(&file);
            let element = effective_key(&file);
            let raw = raw_key(&file);
            match expect {
                Expect::Key(want) => {
                    assert_eq!(element[..], want[..], "{name}");
                    assert!(raw.is_some(), "{name}");
                }
                Expect::Hashed => {
                    assert_eq!(element[..], Sha256::digest(&file)[..], "{name}");
                    assert!(raw.is_some(), "{name}");
                }
                Expect::NotRaw => assert_eq!(raw, None, "{name}"),
            }
            if let Some(raw) = raw {
                assert_eq!(raw[..], element[..], "{name}");
                assert!(opens(&kdbx, &raw), "{name}: password + raw_key opens it");
            } else if element.len() == 32 {
                assert!(
                    !opens(&kdbx, &element),
                    "{name}: raw_key is None only when a 32-byte file of the key reads differently"
                );
            }
            assert!(
                opens(&kdbx, &v1_xml(&element)),
                "{name}: password + effective_key opens it"
            );
        }
    }

    /// The audit's M5 case: KeePass hashes a 65-byte file of 64 hex digits plus a newline, so the
    /// decoded hex (what the old trimming parser returned) does not open the database.
    #[test]
    fn hex_with_newline_is_hashed_not_decoded() {
        let k = [0x5A; 32];
        let file = format!("{}\n", hex::encode(k)).into_bytes();
        assert_eq!(file.len(), 65);
        let kdbx = keyed_with(&file);
        assert!(!opens(&kdbx, &k));
        let raw = raw_key(&file).unwrap();
        assert_ne!(raw, k);
        assert!(opens(&kdbx, &raw));
    }
}
