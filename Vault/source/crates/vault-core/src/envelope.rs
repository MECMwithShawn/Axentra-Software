//! The `<database>.axvault` sidecar: a 32-byte database key wrapped under a passkey's PRF secret.
//!
//! Byte-compatible with the KeePass plugin (`keepass-plugin/src/Envelope.cs`):
//! HKDF-SHA256(ikm = PRF output, salt = PRF input) -> 32-byte AES key || 32-byte HMAC key,
//! AES-256-CBC/PKCS7 over the key, then HMAC-SHA256 over every other field (encrypt-then-MAC).

use std::path::{Path, PathBuf};

use aes::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::random_bytes;

pub const MAGIC: &str = "AxentraVault/1";
pub const EXTENSION: &str = ".axvault";
const HKDF_INFO: &[u8] = b"axentra-vault/v1 aes-256-cbc+hmac-sha256";

/// Longest credential id WebAuthn allows, in bytes.
pub const MAX_CREDENTIAL_ID: usize = 1023;
/// Longest relying-party id: a DNS name.
pub const MAX_RP_ID: usize = 253;
/// Most unlock keys one passkey file may hold.
pub const MAX_KEYS: usize = 32;
/// Longest key label, in characters.
pub const MAX_LABEL: usize = 128;
/// The wrapped 32-byte key plus a full PKCS7 padding block.
const CIPHERTEXT_LEN: usize = 48;

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    #[error("not an Axentra Vault passkey file (expected {MAGIC})")]
    NotAnEnvelope,
    #[error("passkey file is missing '{0}'")]
    MissingField(&'static str),
    #[error("passkey file field '{0}' is malformed")]
    BadField(&'static str),
    #[error("passkey file repeats the field '{0}'")]
    DuplicateField(String),
    #[error("passkey file lists more than {MAX_KEYS} keys")]
    TooManyKeys,
    #[error("passkey file lists the same key twice")]
    DuplicateKey,
    #[error("the passkey secret does not match this database's passkey file")]
    WrongSecret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub rp_id: String,
    pub credential_id: Vec<u8>,
    pub prf_input: [u8; 32],
    iv: [u8; 16],
    ciphertext: Vec<u8>,
    mac: [u8; 32],
}

/// `<database path>.axvault`, next to the database.
pub fn path_for(database: &Path) -> PathBuf {
    let mut s = database.as_os_str().to_owned();
    s.push(EXTENSION);
    PathBuf::from(s)
}

/// `<database path>.axvault.~new`: a key change's new ring, staged beside the database before the
/// database commits to the new key and moved over the passkey file after. Found at the next unlock
/// if the app stopped in between.
pub fn staged_path_for(database: &Path) -> PathBuf {
    let mut s = path_for(database).into_os_string();
    s.push(".~new");
    PathBuf::from(s)
}

/// `<database path>.axvault.old`: the previous ring that earlier versions (and the KeePass plugin)
/// kept. It may wrap a revoked key; Axentra never writes it and deletes it at a key change.
pub fn legacy_old_path_for(database: &Path) -> PathBuf {
    let mut s = path_for(database).into_os_string();
    s.push(".old");
    PathBuf::from(s)
}

/// Replaces the passkey file `env_path` with `ring` all at once: a uniquely named temp file
/// beside it (created new, flushed), then a rename over it that returns once it is on disk. No
/// copy of the ring it replaces is kept. A ring that unlocking would refuse is never written.
/// Returns the text written.
pub fn write_ring(env_path: &Path, ring: &KeyRing) -> std::io::Result<String> {
    let text = ring.serialize();
    write_ring_text(env_path, &text)?;
    Ok(text)
}

/// [`write_ring`] for a ring already serialized (a snapshot being put back).
pub fn write_ring_text(env_path: &Path, text: &str) -> std::io::Result<()> {
    KeyRing::parse(text).map_err(std::io::Error::other)?;
    let tmp = crate::fsops::write_temp_beside(env_path, text.as_bytes())?;
    crate::fsops::move_replace(&tmp, env_path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Stages `ring` as `<database>.axvault.~new` (see [`staged_path_for`]), replacing a leftover
/// from an earlier key change that never finished. Returns the text written.
pub fn stage_ring(database: &Path, ring: &KeyRing) -> std::io::Result<String> {
    let text = ring.serialize();
    KeyRing::parse(&text).map_err(std::io::Error::other)?;
    let staged = staged_path_for(database);
    crate::fsops::remove_file(&staged)?;
    crate::fsops::write_new(&staged, text.as_bytes())?;
    Ok(text)
}

/// Reads a passkey file as text, capped at [`limits::SIDECAR_MAX`](crate::limits::SIDECAR_MAX):
/// even a full key ring is a few kilobytes.
pub fn read_text(path: &Path) -> std::io::Result<String> {
    let bytes = crate::limits::read_capped(path, crate::limits::SIDECAR_MAX)?;
    std::str::from_utf8(&bytes).map(str::to_owned).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the passkey file is not text",
        )
    })
}

impl Envelope {
    pub fn seal(
        rp_id: &str,
        credential_id: &[u8],
        prf_input: [u8; 32],
        prf_output: &[u8; 32],
        key: &[u8; 32],
    ) -> Self {
        let mut iv = [0u8; 16];
        random_bytes(&mut iv);
        let (enc_key, mac_key) = derive_keys(prf_output, &prf_input);

        // 32 bytes of key + a full PKCS7 padding block.
        let mut buf = Zeroizing::new([0u8; 48]);
        let ciphertext = Aes256CbcEnc::new((&*enc_key).into(), (&iv).into())
            .encrypt_padded_b2b::<Pkcs7>(key, &mut buf[..])
            .expect("48-byte buffer fits 32 bytes plus padding")
            .to_vec();

        let mut env = Envelope {
            rp_id: rp_id.to_owned(),
            credential_id: credential_id.to_vec(),
            prf_input,
            iv,
            ciphertext,
            mac: [0u8; 32],
        };
        env.mac = env.mac_for(&mac_key).finalize().into_bytes().into();
        env
    }

    /// Unwraps the database key. Fails if the PRF output is wrong or any field was modified.
    pub fn open(&self, prf_output: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>, EnvelopeError> {
        let (enc_key, mac_key) = derive_keys(prf_output, &self.prf_input);
        self.mac_for(&mac_key)
            .verify_slice(&self.mac)
            .map_err(|_| EnvelopeError::WrongSecret)?;

        let mut buf = Zeroizing::new(self.ciphertext.clone());
        let plain = Aes256CbcDec::new((&*enc_key).into(), (&self.iv).into())
            .decrypt_padded::<Pkcs7>(&mut buf[..])
            .map_err(|_| EnvelopeError::WrongSecret)?;
        let key: [u8; 32] = plain
            .try_into()
            .map_err(|_| EnvelopeError::BadField("ciphertext"))?;
        Ok(Zeroizing::new(key))
    }

    pub fn serialize(&self) -> String {
        format!(
            "{MAGIC}\nrpId={}\ncredentialId={}\nprfInput={}\niv={}\nciphertext={}\nmac={}\n",
            self.rp_id,
            B64.encode(&self.credential_id),
            B64.encode(self.prf_input),
            B64.encode(self.iv),
            B64.encode(&self.ciphertext),
            B64.encode(self.mac),
        )
    }

    pub fn parse(text: &str) -> Result<Self, EnvelopeError> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        if lines.next() != Some(MAGIC) {
            return Err(EnvelopeError::NotAnEnvelope);
        }
        let mut fields = std::collections::HashMap::new();
        for line in lines {
            if let Some((k, v)) = line.split_once('=') {
                // Readers that keep the first or the last copy would disagree; refuse both.
                if fields.insert(k.trim(), v.trim()).is_some() {
                    let name: String = k.trim().chars().take(40).collect();
                    return Err(EnvelopeError::DuplicateField(name));
                }
            }
        }
        let field = |name: &'static str| -> Result<&str, EnvelopeError> {
            fields
                .get(name)
                .copied()
                .filter(|v| !v.is_empty())
                .ok_or(EnvelopeError::MissingField(name))
        };
        // `max` bounds the decoded length; the text is checked first, so nothing large is decoded.
        let bytes = |name: &'static str, max: usize| -> Result<Vec<u8>, EnvelopeError> {
            let text = field(name)?;
            if text.len() > max.div_ceil(3) * 4 {
                return Err(EnvelopeError::BadField(name));
            }
            B64.decode(text)
                .ok()
                .filter(|b| (1..=max).contains(&b.len()))
                .ok_or(EnvelopeError::BadField(name))
        };

        let rp_id = field("rpId")?;
        if !rp_id.is_ascii() || rp_id.len() > MAX_RP_ID {
            return Err(EnvelopeError::BadField("rpId"));
        }
        let ciphertext = bytes("ciphertext", CIPHERTEXT_LEN)?;
        if ciphertext.len() != CIPHERTEXT_LEN {
            return Err(EnvelopeError::BadField("ciphertext"));
        }
        Ok(Envelope {
            rp_id: rp_id.to_owned(),
            credential_id: bytes("credentialId", MAX_CREDENTIAL_ID)?,
            prf_input: bytes("prfInput", 32)?
                .try_into()
                .map_err(|_| EnvelopeError::BadField("prfInput"))?,
            iv: bytes("iv", 16)?
                .try_into()
                .map_err(|_| EnvelopeError::BadField("iv"))?,
            ciphertext,
            mac: bytes("mac", 32)?
                .try_into()
                .map_err(|_| EnvelopeError::BadField("mac"))?,
        })
    }

    /// HMAC over length-prefixed fields; lengths are little-endian i32 to match .NET BitConverter.
    fn mac_for(&self, mac_key: &[u8; 32]) -> Hmac<Sha256> {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(mac_key)
            .expect("HMAC accepts any key length");
        for field in [
            MAGIC.as_bytes(),
            self.rp_id.as_bytes(),
            &self.credential_id,
            &self.prf_input,
            &self.iv,
            &self.ciphertext,
        ] {
            mac.update(&(field.len() as i32).to_le_bytes());
            mac.update(field);
        }
        mac
    }
}

/// Header of a multi-key file. A single key is still written in the v1 form (with an extra
/// `label=` line, which v1 readers such as the KeePass plugin ignore), so one-key vaults stay
/// readable everywhere.
pub const MAGIC_V2: &str = "AxentraVault/2";

/// One unlock key: a label for people ("Security key (USB)") and its wrapped copy of the
/// database key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RingKey {
    pub label: String,
    pub envelope: Envelope,
}

/// Every passkey / security key that can unlock a database. All of them wrap the same 32-byte
/// database key, so any one of them (or the backup key file) opens it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyRing {
    pub keys: Vec<RingKey>,
}

impl KeyRing {
    pub fn parse(text: &str) -> Result<Self, EnvelopeError> {
        let first = text.lines().map(str::trim).find(|l| !l.is_empty());
        let label_of = |block: &str| {
            block
                .lines()
                .filter_map(|l| l.trim().strip_prefix("label="))
                .next()
                .map(|l| l.trim().to_owned())
                .unwrap_or_else(|| "Passkey".to_owned())
        };
        let ring = match first {
            Some(MAGIC) => KeyRing {
                keys: vec![RingKey {
                    label: label_of(text),
                    envelope: Envelope::parse(text)?,
                }],
            },
            Some(MAGIC_V2) => {
                let blocks = text.split("[key]").skip(1);
                if blocks.clone().nth(MAX_KEYS).is_some() {
                    return Err(EnvelopeError::TooManyKeys);
                }
                let mut keys = Vec::new();
                for block in blocks {
                    let envelope = Envelope::parse(&format!("{MAGIC}\n{block}"))?;
                    keys.push(RingKey {
                        label: label_of(block),
                        envelope,
                    });
                }
                if keys.is_empty() {
                    return Err(EnvelopeError::MissingField("[key]"));
                }
                KeyRing { keys }
            }
            _ => return Err(EnvelopeError::NotAnEnvelope),
        };
        if ring
            .keys
            .iter()
            .any(|k| k.label.chars().count() > MAX_LABEL)
        {
            return Err(EnvelopeError::BadField("label"));
        }
        // `find` returns the first match; a second key with the same id could never be used.
        let mut ids = std::collections::HashSet::new();
        if !ring
            .keys
            .iter()
            .all(|k| ids.insert(&k.envelope.credential_id))
        {
            return Err(EnvelopeError::DuplicateKey);
        }
        Ok(ring)
    }

    pub fn serialize(&self) -> String {
        let body = |k: &RingKey| {
            let v1 = k.envelope.serialize();
            let fields = v1.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            // One line, and never the block separator.
            let label = k.label.replace(['\r', '\n'], " ").replace("[key]", "key");
            format!("label={label}\n{fields}")
        };
        match self.keys.as_slice() {
            [one] => format!("{MAGIC}\n{}", body(one)),
            keys => {
                let mut out = format!("{MAGIC_V2}\n");
                for k in keys {
                    out.push_str("\n[key]\n");
                    out.push_str(&body(k));
                }
                out
            }
        }
    }

    /// The key whose credential id matches (as reported by the authenticator).
    pub fn find(&self, credential_id: &[u8]) -> Option<&RingKey> {
        self.keys
            .iter()
            .find(|k| k.envelope.credential_id == credential_id)
    }
}

fn derive_keys(
    prf_output: &[u8; 32],
    prf_input: &[u8; 32],
) -> (Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>) {
    let mut okm = Zeroizing::new([0u8; 64]);
    Hkdf::<Sha256>::new(Some(prf_input), prf_output)
        .expand(HKDF_INFO, &mut okm[..])
        .expect("64 bytes is a valid HKDF-SHA256 length");
    let mut enc = Zeroizing::new([0u8; 32]);
    let mut mac = Zeroizing::new([0u8; 32]);
    enc.copy_from_slice(&okm[..32]);
    mac.copy_from_slice(&okm[32..]);
    (enc, mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Envelope, [u8; 32], [u8; 32]) {
        let mut prf = [0u8; 32];
        let mut key = [0u8; 32];
        let mut input = [0u8; 32];
        random_bytes(&mut prf);
        random_bytes(&mut key);
        random_bytes(&mut input);
        (
            Envelope::seal("localhost", &[1, 2, 3, 4], input, &prf, &key),
            prf,
            key,
        )
    }

    #[test]
    fn round_trip_through_text() {
        let (env, prf, key) = sample();
        let parsed = Envelope::parse(&env.serialize()).unwrap();
        assert_eq!(parsed, env);
        assert_eq!(*parsed.open(&prf).unwrap(), key);
    }

    #[test]
    fn wrong_secret_rejected() {
        let (env, mut prf, _) = sample();
        prf[0] ^= 1;
        assert!(matches!(env.open(&prf), Err(EnvelopeError::WrongSecret)));
    }

    #[test]
    fn every_field_is_authenticated() {
        let (env, prf, _) = sample();
        let tampers: [fn(&mut Envelope); 6] = [
            |e| e.rp_id.push('x'),
            |e| e.credential_id[0] ^= 1,
            |e| e.prf_input[0] ^= 1,
            |e| e.iv[0] ^= 1,
            |e| e.ciphertext[0] ^= 1,
            |e| e.mac[0] ^= 1,
        ];
        for tamper in tampers {
            let mut e = env.clone();
            tamper(&mut e);
            assert!(e.open(&prf).is_err());
        }
    }

    #[test]
    fn rejects_foreign_text() {
        assert!(matches!(
            Envelope::parse(""),
            Err(EnvelopeError::NotAnEnvelope)
        ));
        assert!(matches!(
            Envelope::parse("hello\nrpId=x"),
            Err(EnvelopeError::NotAnEnvelope)
        ));
        assert!(matches!(
            Envelope::parse(&format!("{MAGIC}\nrpId=x")),
            Err(EnvelopeError::MissingField(_))
        ));
    }

    #[test]
    fn key_ring_round_trips_and_stays_v1_with_one_key() {
        let (a, prf_a, key) = sample();
        let mut ring = KeyRing {
            keys: vec![RingKey {
                label: "Phone passkey".into(),
                envelope: a.clone(),
            }],
        };
        let one = ring.serialize();
        assert!(one.starts_with(MAGIC), "one key keeps the v1 format");
        assert_eq!(
            Envelope::parse(&one).unwrap(),
            a,
            "v1 readers (KeePass plugin) still parse it"
        );
        assert_eq!(KeyRing::parse(&one).unwrap(), ring);

        let mut prf_b = [0u8; 32];
        let mut input_b = [0u8; 32];
        random_bytes(&mut prf_b);
        random_bytes(&mut input_b);
        let b = Envelope::seal("localhost", &[9, 9, 9], input_b, &prf_b, &key);
        ring.keys.push(RingKey {
            label: "Security key (USB)".into(),
            envelope: b,
        });
        let two = ring.serialize();
        assert!(two.starts_with(MAGIC_V2));
        let back = KeyRing::parse(&two).unwrap();
        assert_eq!(back, ring);
        // Either key opens the same database key.
        assert_eq!(
            *back
                .find(&[1, 2, 3, 4])
                .unwrap()
                .envelope
                .open(&prf_a)
                .unwrap(),
            key
        );
        assert_eq!(
            *back
                .find(&[9, 9, 9])
                .unwrap()
                .envelope
                .open(&prf_b)
                .unwrap(),
            key
        );
    }

    /// `text` with the value of field `name` replaced.
    fn with_field(text: &str, name: &str, value: &str) -> String {
        let prefix = format!("{name}=");
        text.lines()
            .map(|l| {
                if l.starts_with(&prefix) {
                    format!("{prefix}{value}")
                } else {
                    l.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn repeated_fields_are_rejected() {
        let (env, _, _) = sample();
        // Either copy alone would parse; which one a reader keeps would decide what it uses.
        let iv = format!("iv={}\n", B64.encode([0u8; 16]));
        assert!(matches!(
            Envelope::parse(&format!("{}{iv}", env.serialize())),
            Err(EnvelopeError::DuplicateField(f)) if f == "iv"
        ));
        let one = KeyRing {
            keys: vec![RingKey {
                label: "Phone passkey".into(),
                envelope: env,
            }],
        }
        .serialize();
        assert!(matches!(
            KeyRing::parse(&format!("{one}label=Another\n")),
            Err(EnvelopeError::DuplicateField(f)) if f == "label"
        ));
    }

    #[test]
    fn credential_id_length_is_bounded() {
        let text = sample().0.serialize();
        let with_id = |n: usize| with_field(&text, "credentialId", &B64.encode(vec![7u8; n]));
        let parsed = Envelope::parse(&with_id(MAX_CREDENTIAL_ID)).unwrap();
        assert_eq!(parsed.credential_id.len(), MAX_CREDENTIAL_ID);
        for bad in [
            with_id(MAX_CREDENTIAL_ID + 1),
            // A megabyte of base64 is refused by its length, before it is decoded.
            with_id(1 << 20),
            with_field(&text, "credentialId", "===="),
        ] {
            assert!(matches!(
                Envelope::parse(&bad),
                Err(EnvelopeError::BadField("credentialId"))
            ));
        }
        assert!(matches!(
            Envelope::parse(&with_field(&text, "credentialId", "")),
            Err(EnvelopeError::MissingField("credentialId"))
        ));
    }

    #[test]
    fn ciphertext_must_be_exactly_one_wrapped_key() {
        let text = sample().0.serialize();
        for n in [16, 32, 47, 49, 64, 4096] {
            let bad = with_field(&text, "ciphertext", &B64.encode(vec![1u8; n]));
            assert!(
                matches!(
                    Envelope::parse(&bad),
                    Err(EnvelopeError::BadField("ciphertext"))
                ),
                "{n}-byte ciphertext accepted"
            );
        }
    }

    #[test]
    fn rp_id_must_be_a_short_ascii_name() {
        let text = sample().0.serialize();
        let longest = "a".repeat(MAX_RP_ID);
        assert_eq!(
            Envelope::parse(&with_field(&text, "rpId", &longest))
                .unwrap()
                .rp_id,
            longest
        );
        // The second is "localhost" with a Cyrillic "о".
        for bad in ["a".repeat(MAX_RP_ID + 1), "l\u{43e}calhost".into()] {
            assert!(matches!(
                Envelope::parse(&with_field(&text, "rpId", &bad)),
                Err(EnvelopeError::BadField("rpId"))
            ));
        }
    }

    #[test]
    fn key_ring_limits() {
        let ring = |n: usize| KeyRing {
            keys: (1..=n)
                .map(|i| RingKey {
                    label: format!("Key {i}"),
                    envelope: Envelope::seal("localhost", &[i as u8], [1; 32], &[2; 32], &[3; 32]),
                })
                .collect(),
        };
        let full = ring(MAX_KEYS);
        assert_eq!(KeyRing::parse(&full.serialize()).unwrap(), full);
        assert!(matches!(
            KeyRing::parse(&ring(MAX_KEYS + 1).serialize()),
            Err(EnvelopeError::TooManyKeys)
        ));

        let mut twice = ring(2);
        twice.keys[1].envelope = twice.keys[0].envelope.clone();
        assert!(matches!(
            KeyRing::parse(&twice.serialize()),
            Err(EnvelopeError::DuplicateKey)
        ));

        // Characters, not bytes: 128 two-byte characters fit.
        for (keys, label, ok) in [
            (1, "é".repeat(MAX_LABEL), true),
            (1, "é".repeat(MAX_LABEL + 1), false),
            (2, "x".repeat(MAX_LABEL + 1), false),
        ] {
            let mut r = ring(keys);
            r.keys[0].label = label;
            let parsed = KeyRing::parse(&r.serialize());
            assert_eq!(parsed.is_ok(), ok, "{keys} key(s)");
            if !ok {
                assert!(matches!(parsed, Err(EnvelopeError::BadField("label"))));
            }
        }
    }

    #[test]
    fn path_appends_extension() {
        assert_eq!(
            path_for(Path::new(r"C:\a\Shawn.kdbx")),
            PathBuf::from(r"C:\a\Shawn.kdbx.axvault")
        );
        assert_eq!(
            staged_path_for(Path::new(r"C:\a\Shawn.kdbx")),
            PathBuf::from(r"C:\a\Shawn.kdbx.axvault.~new")
        );
        assert_eq!(
            legacy_old_path_for(Path::new(r"C:\a\Shawn.kdbx")),
            PathBuf::from(r"C:\a\Shawn.kdbx.axvault.old")
        );
    }

    /// The passkey file is replaced whole, keeps no `.old` copy and leaves no temp file; a ring
    /// that unlocking would refuse is never written; staging replaces a leftover.
    #[test]
    fn ring_files_are_replaced_whole_and_never_copied() {
        let mut n = [0u8; 8];
        random_bytes(&mut n);
        let dir = std::env::temp_dir().join(format!("axvault-ring-{}", hex::encode(n)));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("v.kdbx");
        let env = path_for(&db);
        let ring = |id: u8| KeyRing {
            keys: vec![RingKey {
                label: format!("Key {id}"),
                envelope: Envelope::seal("localhost", &[id], [1; 32], &[id; 32], &[2; 32]),
            }],
        };
        let first = write_ring(&env, &ring(1)).unwrap();
        assert_eq!(std::fs::read_to_string(&env).unwrap(), first);
        let second = write_ring(&env, &ring(2)).unwrap();
        assert_eq!(std::fs::read_to_string(&env).unwrap(), second);

        let mut bad = ring(3);
        bad.keys[0].label = "x".repeat(MAX_LABEL + 1);
        assert!(write_ring(&env, &bad).is_err());
        assert!(write_ring_text(&env, "not a ring").is_err());
        assert_eq!(std::fs::read_to_string(&env).unwrap(), second);

        std::fs::write(staged_path_for(&db), "leftover").unwrap();
        let staged = stage_ring(&db, &ring(4)).unwrap();
        assert_eq!(
            std::fs::read_to_string(staged_path_for(&db)).unwrap(),
            staged
        );

        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["v.kdbx.axvault", "v.kdbx.axvault.~new"]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
