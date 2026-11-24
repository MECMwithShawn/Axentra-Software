//! Save-fidelity check: open a database, save it in memory with this app's writer, and compare
//! the decrypted XML before and after. Any element or value that does not survive a save shows up
//! as a difference. Values are reported only as short hashes, so the report holds no secrets.
//!
//! Equivalent spellings are normalized first: empty elements (KeePass treats empty and absent
//! alike), `Null`/`null`, and `QualityCheck=True` (the default). This app's own credential record
//! (Meta CustomData `AxentraVault.Credentials`, see [`crate::generation`]) is left out: a re-key
//! replaces it on purpose.
//!
//! [`check`] saves a database and compares; [`compare`] compares two decrypted XML documents, such
//! as a backup and its copy re-encrypted to another key.

use std::collections::BTreeMap;

use keepass::{Database, DatabaseKey};
use quick_xml::{XmlVersion, events::Event, reader::Reader};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum FidelityError {
    #[error("could not open the database: {0}")]
    Open(#[from] keepass::db::DatabaseOpenError),
    #[error("could not save the database: {0}")]
    Save(#[from] keepass::db::DatabaseSaveError),
    #[error("could not read the database XML: {0}")]
    Xml(String),
}

#[derive(Debug, Default)]
pub struct FidelityReport {
    pub elements_before: usize,
    pub elements_after: usize,
    /// `(path, value hash, count before, count after)` for every mismatch.
    pub differences: Vec<(String, String, usize, usize)>,
    /// Built-in fallback icons dropped where a custom icon is set (the custom icon is kept).
    pub fallback_icons_dropped: usize,
}

impl FidelityReport {
    pub fn is_clean(&self) -> bool {
        self.differences.is_empty()
    }
}

/// Elements the writer is expected to change on every save.
const EXPECTED_TO_CHANGE: &[&str] = &["KeePassFile/Meta/Generator", "KeePassFile/Meta/HeaderHash"];

pub fn check(data: &[u8], key: DatabaseKey) -> Result<FidelityReport, FidelityError> {
    let mut db = Database::parse(data, key.clone())?;
    let mut saved = Zeroizing::new(Vec::new());
    let result = db.save(&mut *saved, key.clone());
    crate::generation::wipe_record(&mut db);
    result?;

    let before = Zeroizing::new(Database::get_xml(&mut &data[..], key.clone())?);
    let after = Zeroizing::new(Database::get_xml(&mut &saved[..], key)?);
    compare(&before, &after)
}

/// Compares two decrypted KDBX XML documents: `before` (the original) and `after` (as this app's
/// writer saved it). Built-in icons dropped where a custom icon is set are counted, not listed.
pub fn compare(before: &[u8], after: &[u8]) -> Result<FidelityReport, FidelityError> {
    let a = flatten(before)?;
    let b = flatten(after)?;

    let mut report = FidelityReport {
        elements_before: a.values().sum(),
        elements_after: b.values().sum(),
        ..Default::default()
    };
    let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
    for k in keys {
        if EXPECTED_TO_CHANGE.contains(&k.0.as_str()) {
            continue;
        }
        let (ca, cb) = (
            a.get(k).copied().unwrap_or(0),
            b.get(k).copied().unwrap_or(0),
        );
        if ca == cb {
            continue;
        }
        if k.0.ends_with("/IconID") && cb < ca {
            report.fallback_icons_dropped += ca - cb;
        } else {
            report.differences.push((k.0.clone(), k.1.clone(), ca, cb));
        }
    }
    Ok(report)
}

/// The Meta CustomData item that holds the credential record.
const CUSTOM_DATA_ITEM: &str = "KeePassFile/Meta/CustomData/Item";

/// Multiset of (element path, short hash of its text). Protected values are encrypted with a
/// per-save stream key, so only their presence is compared. The credential record's item is
/// left out.
fn flatten(xml: &[u8]) -> Result<BTreeMap<(String, String), usize>, FidelityError> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut version = XmlVersion::Implicit1_0;
    let mut stack: Vec<(String, bool, String)> = Vec::new(); // (name, protected, text)
    let mut out = BTreeMap::new();
    // Inside a Meta CustomData item: what it holds so far, and whether it is the record.
    let mut item: Option<(Vec<(String, String)>, bool)> = None;
    let err = |e: &dyn std::fmt::Display| FidelityError::Xml(e.to_string());

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf).map_err(|e| err(&e))? {
            Event::Eof => break,
            Event::Decl(d) => version = d.xml_version().map_err(|e| err(&e))?,
            Event::Start(e) => {
                let protected = e
                    .attributes()
                    .flatten()
                    .any(|a| a.key.as_ref() == "Protected" && a.value.as_ref() == "True");
                stack.push((e.name().as_ref().to_string(), protected, String::new()));
                let names = stack.iter().map(|s| s.0.as_str());
                if names.eq(CUSTOM_DATA_ITEM.split('/')) {
                    item = Some((Vec::new(), false));
                }
            }
            Event::Empty(_) => {}
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.2.push_str(&t.xml_content(version));
                }
            }
            Event::GeneralRef(r) => {
                if let Some(top) = stack.last_mut() {
                    top.2.push('&');
                    top.2.push_str(&r.xml_content(version));
                    top.2.push(';');
                }
            }
            Event::CData(c) => {
                if let Some(top) = stack.last_mut() {
                    top.2.push_str(&c.into_inner());
                }
            }
            Event::End(_) => {
                let path = path_of(&stack, None);
                let (_, protected, text) = stack
                    .pop()
                    .ok_or_else(|| FidelityError::Xml("unbalanced XML".into()))?;
                let text = text.trim();
                if path == CUSTOM_DATA_ITEM {
                    if let Some((entries, false)) = item.take() {
                        for key in entries {
                            *out.entry(key).or_insert(0) += 1;
                        }
                    }
                    continue;
                }
                if let Some((_, is_record)) = &mut item
                    && path == format!("{CUSTOM_DATA_ITEM}/Key")
                    && text == crate::generation::RECORD_KEY
                {
                    *is_record = true;
                }
                if !protected
                    && (text.is_empty() || (path.ends_with("/QualityCheck") && text == "True"))
                {
                    continue;
                }
                let value = if protected {
                    "<protected>".to_owned()
                } else if text.eq_ignore_ascii_case("null") {
                    "null".to_owned()
                } else {
                    hex::encode(&Sha256::digest(text.as_bytes())[..4])
                };
                match &mut item {
                    Some((entries, _)) => entries.push((path, value)),
                    None => *out.entry((path, value)).or_insert(0) += 1,
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn path_of(stack: &[(String, bool, String)], leaf: Option<&str>) -> String {
    let mut parts: Vec<&str> = stack.iter().map(|s| s.0.as_str()).collect();
    if let Some(l) = leaf {
        parts.push(l);
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use keepass::db::fields;

    use super::*;

    #[test]
    fn own_output_is_clean() {
        let mut db = Database::new();
        db.config.kdf_config = keepass::config::KdfConfig::Aes { rounds: 1000 };
        let mut root = db.root_mut();
        let mut g = root.add_group();
        g.name = "Banking & Finance".into();
        g.add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "Bank <main>");
            e.set_protected(fields::PASSWORD, "hunter2");
            e.set_unprotected(fields::NOTES, "line1\nline2 & more");
            e.tags.push("money".into());
        });
        let key = DatabaseKey::new().with_password("pw");
        let mut data = Vec::new();
        db.save(&mut data, key.clone()).unwrap();

        let report = check(&data, key).unwrap();
        assert!(report.elements_before > 20);
        assert!(report.is_clean(), "{:?}", report.differences);
    }

    fn doc(record: &str, plugin: Option<&str>) -> Vec<u8> {
        let plugin = plugin
            .map(|v| format!("<Item><Key>Plugin.Setting</Key><Value>{v}</Value></Item>"))
            .unwrap_or_default();
        format!(
            "<KeePassFile><Meta><Generator>x</Generator><CustomData>\
             <Item><Key>AxentraVault.Credentials</Key><Value>{record}</Value>\
             <LastModificationTime>{record}</LastModificationTime></Item>{plugin}\
             </CustomData></Meta><Root><Group><Name>Root</Name></Group></Root></KeePassFile>"
        )
        .into_bytes()
    }

    /// A re-key replaces the credential record; anything else that differs, another plugin's
    /// CustomData included, is reported.
    #[test]
    fn compare_ignores_only_the_credential_record() {
        let a = doc("v1;old", Some("kept"));
        let report = compare(&a, &doc("v1;new", Some("kept"))).unwrap();
        assert!(report.is_clean(), "{:?}", report.differences);
        assert!(compare(&a, &doc("", Some("kept"))).unwrap().is_clean());

        let changed = compare(&a, &doc("v1;old", Some("changed"))).unwrap();
        assert_eq!(changed.differences.len(), 2, "{:?}", changed.differences);
        let dropped = compare(&a, &doc("v1;new", None)).unwrap();
        let paths: Vec<_> = dropped.differences.iter().map(|d| d.0.as_str()).collect();
        assert_eq!(
            paths,
            [
                "KeePassFile/Meta/CustomData/Item/Key",
                "KeePassFile/Meta/CustomData/Item/Value"
            ]
        );
        assert!(dropped.differences.iter().all(|d| d.2 == 1 && d.3 == 0));
    }
}
