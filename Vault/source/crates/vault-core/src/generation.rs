//! Credential generations. Every set of credentials a vault is saved under gets a random id, its
//! generation, which the backup store labels copies with. The database itself records the
//! generation in its Meta CustomData item `AxentraVault.Credentials`:
//! `v1;<generation>;<salt>;<check>` (16, 32 and 32 bytes, hex), where
//! `check = HMAC-SHA256(salt, "axentra-vault/credential-check/v1" ‖ composite key)`.
//!
//! At open, a check that matches the credentials that opened the file means they are the ones
//! Axentra last saved it with; one that doesn't means the key was changed outside Axentra (in
//! KeePass, KeePassXC or the plugin). The composite key is KeePass's own, so the same key gives
//! the same check whether it arrives as raw bytes, hex or a `.keyx` file. The check would let
//! anyone holding it test passwords without the KDF, so it never leaves the encrypted database:
//! the store keeps only generations, and copies in memory are wiped.
//!
//! Other KeePass apps keep Meta CustomData items they don't know and otherwise ignore them. One
//! that drops the record only makes the next open start a provisional generation.

use hmac::{Hmac, KeyInit, Mac};
use keepass::Database;
use keepass::db::{CustomDataItem, CustomDataValue, Times};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::keyfile;
use crate::random_bytes;
use crate::vault::Credentials;

/// The Meta CustomData key of the record.
pub const RECORD_KEY: &str = "AxentraVault.Credentials";
const RECORD_VERSION: &str = "v1";
const CHECK_LABEL: &[u8] = b"axentra-vault/credential-check/v1";

/// KeePass's composite key for `credentials`, as KDBX 4 derives it before the KDF: SHA-256 over
/// the SHA-256 of the password (if any) followed by the key-file element (if any), which is
/// [`keyfile::effective_key`] of the key file's contents. With neither factor it is the hash of
/// nothing; KeePass refuses such a key, and so does [`Credentials::to_key`].
pub fn composite_key(credentials: &Credentials) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(Sha256::digest(&key_elements(credentials)[..]).into())
}

/// The key elements keepass hashes into the composite key, concatenated.
fn key_elements(credentials: &Credentials) -> Zeroizing<Vec<u8>> {
    let key_file = credentials
        .key_file
        .as_ref()
        .map(|k| keyfile::effective_key(k));
    // Sized up front, so no regrowth leaves a copy behind.
    let mut out = Zeroizing::new(Vec::with_capacity(
        32 + key_file.as_ref().map_or(0, |k| k.len()),
    ));
    if let Some(password) = &credentials.password {
        out.extend_from_slice(&Sha256::digest(password.as_bytes()));
    }
    if let Some(element) = &key_file {
        out.extend_from_slice(element);
    }
    out
}

fn check_mac(salt: &[u8; 32], credentials: &Credentials) -> Hmac<Sha256> {
    let mut mac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(salt).expect("HMAC accepts any key length");
    mac.update(CHECK_LABEL);
    mac.update(&composite_key(credentials)[..]);
    mac
}

fn random_generation() -> String {
    let mut id = [0u8; 16];
    random_bytes(&mut id);
    hex::encode(id)
}

/// One generation's record: its id, and the salted check of the credentials it was made for.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialRecord {
    generation: String,
    salt: [u8; 32],
    check: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for CredentialRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialRecord")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl CredentialRecord {
    /// A new generation for `credentials`, with a random id and salt.
    pub fn new(credentials: &Credentials) -> CredentialRecord {
        let mut salt = [0u8; 32];
        random_bytes(&mut salt);
        Self::with_salt(random_generation(), salt, credentials)
    }

    fn with_salt(generation: String, salt: [u8; 32], credentials: &Credentials) -> Self {
        CredentialRecord {
            generation,
            salt,
            check: Zeroizing::new(check_mac(&salt, credentials).finalize().into_bytes().into()),
        }
    }

    /// 32 lowercase hex digits.
    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// Whether `credentials` are the ones this record was made for, in any key-file form
    /// (compared in constant time).
    pub fn matches(&self, credentials: &Credentials) -> bool {
        check_mac(&self.salt, credentials)
            .verify_slice(&self.check[..])
            .is_ok()
    }

    /// `v1;<generation>;<salt>;<check>`. None for anything else, including a later version.
    fn parse(value: &str) -> Option<CredentialRecord> {
        let mut parts = value.split(';');
        let (Some(RECORD_VERSION), Some(generation_hex), Some(salt_hex), Some(check_hex), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return None;
        };
        let mut generation = [0u8; 16];
        let mut salt = [0u8; 32];
        let mut check = Zeroizing::new([0u8; 32]);
        hex::decode_to_slice(generation_hex, &mut generation).ok()?;
        hex::decode_to_slice(salt_hex, &mut salt).ok()?;
        hex::decode_to_slice(check_hex, &mut check[..]).ok()?;
        Some(CredentialRecord {
            generation: hex::encode(generation),
            salt,
            check,
        })
    }

    fn value(&self) -> Zeroizing<String> {
        let check = Zeroizing::new(hex::encode(&self.check[..]));
        let mut value = Zeroizing::new(String::with_capacity(2 + 1 + 32 + 1 + 64 + 1 + 64));
        for part in [
            RECORD_VERSION,
            ";",
            &self.generation,
            ";",
            &hex::encode(self.salt),
            ";",
            &check,
        ] {
            value.push_str(part);
        }
        value
    }

    /// The record in `db`'s Meta CustomData, if there is a well-formed one.
    pub fn read(db: &Database) -> Option<CredentialRecord> {
        match db.meta.custom_data.get(RECORD_KEY)?.value.as_ref()? {
            CustomDataValue::String(value) => Self::parse(value),
            // keepass reads values that decode as base64 as binary; a record never does (`;`).
            CustomDataValue::Binary(_) => None,
        }
    }

    /// Puts this record into `db`'s Meta CustomData, unless it is already there, wiping the value
    /// it replaces. Returns whether anything changed.
    pub fn write_to(&self, db: &mut Database) -> bool {
        let mut value = self.value();
        if matches!(
            db.meta.custom_data.get(RECORD_KEY).and_then(|item| item.value.as_ref()),
            Some(CustomDataValue::String(current)) if *current == *value
        ) {
            return false;
        }
        wipe_record(db);
        db.meta.custom_data.insert(
            RECORD_KEY.to_owned(),
            CustomDataItem {
                // Moved, not copied, into the database.
                value: Some(CustomDataValue::String(std::mem::take(&mut *value))),
                last_modification_time: Some(Times::now()),
            },
        );
        true
    }

    /// The parts that are kept while a vault is locked for recovery: the id and the salt, both
    /// random. The check is recomputed from the credentials that reopen it.
    pub(crate) fn seal(&self) -> SealedRecord {
        SealedRecord {
            generation: self.generation.clone(),
            salt: self.salt,
        }
    }
}

/// A [`CredentialRecord`] without its check, which would test passwords without the KDF.
#[derive(Clone)]
pub(crate) struct SealedRecord {
    generation: String,
    salt: [u8; 32],
}

impl SealedRecord {
    pub(crate) fn unseal(&self, credentials: &Credentials) -> CredentialRecord {
        CredentialRecord::with_salt(self.generation.clone(), self.salt, credentials)
    }
}

/// Zeroizes and removes the record from `db`, for a database about to be dropped or a record
/// about to be replaced.
pub fn wipe_record(db: &mut Database) {
    if let Some(mut item) = db.meta.custom_data.remove(RECORD_KEY) {
        match &mut item.value {
            Some(CustomDataValue::String(s)) => s.zeroize(),
            Some(CustomDataValue::Binary(b)) => b.zeroize(),
            None => {}
        }
    }
}

/// What the record found at open says about the credentials that opened the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordAtOpen {
    /// The record names these credentials: they are the ones Axentra last saved it with.
    Matches,
    /// No (well-formed) record: never saved by Axentra since generations exist, or another app
    /// dropped it.
    Missing,
    /// The record names other credentials: the key was changed outside Axentra.
    Mismatch,
}

/// The session's generation at open: the recorded one if it names `credentials` (committed: the
/// file already carries it), otherwise a new, provisional one.
pub fn at_open(db: &Database, credentials: &Credentials) -> (CredentialRecord, RecordAtOpen) {
    match CredentialRecord::read(db) {
        Some(record) if record.matches(credentials) => (record, RecordAtOpen::Matches),
        Some(_) => (CredentialRecord::new(credentials), RecordAtOpen::Mismatch),
        None => (CredentialRecord::new(credentials), RecordAtOpen::Missing),
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use keepass::DatabaseKey;
    use keepass::config::KdfConfig;

    use super::*;

    fn creds(password: Option<&str>, key_file: Option<&[u8]>) -> Credentials {
        Credentials {
            password: password.map(|p| Zeroizing::new(p.to_owned())),
            key_file: key_file.map(|k| Zeroizing::new(k.to_vec())),
        }
    }

    /// A format 1.0 XML key file whose single key element is `element`, of any length.
    fn v1_xml(element: &[u8]) -> Vec<u8> {
        let data = base64::engine::general_purpose::STANDARD.encode(element);
        format!("<KeyFile><Key><Data>{data}</Data></Key></KeyFile>").into_bytes()
    }

    /// Differential check against keepass: a database keyed with `credentials` opens with a key
    /// made of one element, our concatenated elements, so keepass's composite (the SHA-256 of
    /// its elements) is ours.
    #[test]
    fn composite_key_is_keepass_composite() {
        let k: [u8; 32] = core::array::from_fn(|i| (i as u8).wrapping_mul(29).wrapping_add(3));
        let keyx = keyfile::build_v2(&k).into_bytes();
        let bare = b"a bare key file KeePass hashes".to_vec();
        let table: Vec<(&str, Credentials)> = vec![
            ("password", creds(Some("pw"), None)),
            ("empty password", creds(Some(""), None)),
            ("unicode password", creds(Some("pässwörd 中 🔑"), None)),
            ("raw key file", creds(None, Some(&k))),
            ("password + raw key", creds(Some("pw"), Some(&k))),
            ("password + .keyx", creds(Some("pw"), Some(&keyx))),
            ("password + bare file", creds(Some("pw"), Some(&bare))),
        ];
        for (name, c) in table {
            let mut db = Database::new();
            db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
            let mut kdbx = Vec::new();
            db.save(&mut kdbx, c.to_key().unwrap()).unwrap();
            let elements = key_elements(&c);
            let one_element = DatabaseKey::new()
                .with_keyfile(&mut &v1_xml(&elements)[..])
                .unwrap();
            assert!(Database::parse(&kdbx, one_element).is_ok(), "{name}");
            assert_eq!(
                composite_key(&c)[..],
                Sha256::digest(&elements[..])[..],
                "{name}"
            );
        }
    }

    #[test]
    fn same_key_gives_the_same_check_in_any_form() {
        let k = [0x42u8; 32];
        let forms = [
            creds(Some("pw"), Some(&k)),
            creds(Some("pw"), Some(keyfile::build_v2(&k).as_bytes())),
            creds(Some("pw"), Some(hex::encode(k).as_bytes())),
            creds(Some("pw"), Some(&v1_xml(&k))),
        ];
        let record = CredentialRecord::new(&forms[0]);
        for (i, c) in forms.iter().enumerate() {
            assert_eq!(
                composite_key(c)[..],
                composite_key(&forms[0])[..],
                "form {i}"
            );
            assert!(record.matches(c), "form {i}");
            assert!(
                CredentialRecord::with_salt(record.generation.clone(), record.salt, c) == record,
                "form {i}"
            );
        }
        // A bare file is hashed: its SHA-256 as a raw key file is the same key.
        let bare = b"bare key file".to_vec();
        let hashed: [u8; 32] = Sha256::digest(&bare).into();
        assert!(
            CredentialRecord::new(&creds(Some("pw"), Some(&bare)))
                .matches(&creds(Some("pw"), Some(&hashed)))
        );
        for other in [
            creds(Some("pw2"), Some(&k)),
            creds(Some("pw"), Some(&[0x43; 32])),
            creds(Some("pw"), None),
            creds(None, Some(&k)),
        ] {
            assert!(!record.matches(&other));
        }
    }

    #[test]
    fn record_value_round_trips_and_odd_values_are_missing() {
        let c = creds(Some("pw"), None);
        let record = CredentialRecord::new(&c);
        let value = record.value();
        let parts: Vec<&str> = value.split(';').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "v1");
        assert_eq!(parts[1], record.generation());
        assert_eq!(
            (parts[1].len(), parts[2].len(), parts[3].len()),
            (32, 64, 64)
        );
        assert!(
            value
                .bytes()
                .all(|b| b == b';' || b.is_ascii_alphanumeric())
        );
        assert!(
            base64::engine::general_purpose::STANDARD
                .decode(value.as_bytes())
                .is_err()
        );
        assert!(CredentialRecord::parse(&value) == Some(record.clone()));
        let upper = value.to_ascii_uppercase().replacen("V1", "v1", 1);
        assert!(CredentialRecord::parse(&upper) == Some(record.clone()));

        let (g, s, k) = (parts[1], parts[2], parts[3]);
        for odd in [
            String::new(),
            format!("v2;{g};{s};{k}"),
            format!("v1;{g};{s}"),
            format!("v1;{g};{s};{k};"),
            format!("v1;{g};{s};{k};extra"),
            format!("v1;{};{s};{k}", &g[2..]),
            format!("v1;{g};{s};{}", &k[..62]),
            format!("v1;{g};{s};{}zz", &k[..62]),
        ] {
            assert!(CredentialRecord::parse(&odd).is_none(), "{odd}");
        }
    }

    #[test]
    fn record_is_written_once_and_replaced_values_are_wiped() {
        let c = creds(Some("pw"), None);
        let mut db = Database::new();
        db.meta.custom_data.insert(
            "Other.Plugin".into(),
            CustomDataItem {
                value: Some(CustomDataValue::String("kept".into())),
                last_modification_time: None,
            },
        );
        let (first, found) = at_open(&db, &c);
        assert_eq!(found, RecordAtOpen::Missing);
        assert!(first.write_to(&mut db));
        assert!(
            !first.write_to(&mut db),
            "an identical record is not rewritten"
        );
        assert!(
            db.meta.custom_data[RECORD_KEY]
                .last_modification_time
                .is_some()
        );
        assert!(CredentialRecord::read(&db) == Some(first.clone()));
        assert_eq!(at_open(&db, &c), (first.clone(), RecordAtOpen::Matches));

        let other = CredentialRecord::new(&creds(Some("pw2"), None));
        let (fresh, found) = at_open(&db, &creds(Some("pw2"), None));
        assert_eq!(found, RecordAtOpen::Mismatch);
        assert_ne!(fresh.generation(), first.generation());
        assert!(other.write_to(&mut db));
        assert!(CredentialRecord::read(&db) == Some(other));

        // keepass reads a value that is valid base64 as binary; that is no record of ours.
        db.meta.custom_data.get_mut(RECORD_KEY).unwrap().value =
            Some(CustomDataValue::Binary(vec![1, 2, 3]));
        assert_eq!(at_open(&db, &c).1, RecordAtOpen::Missing);

        wipe_record(&mut db);
        assert!(!db.meta.custom_data.contains_key(RECORD_KEY));
        assert_eq!(
            db.meta.custom_data["Other.Plugin"].value,
            Some(CustomDataValue::String("kept".into()))
        );
    }
}
