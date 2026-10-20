//! The `.axobk` container: a fixed header followed by AES-256-GCM over
//! deflate-compressed JSON.
//!
//! ```text
//! offset  size  field
//!      0     7  magic "AXOBK1\0"
//!      7     2  format version (u16, little endian) = 1
//!      9     1  KDF id (1 = Argon2id v1.3)
//!     10     4  Argon2 memory in KiB (u32 LE)
//!     14     4  Argon2 iterations (u32 LE)
//!     18     4  Argon2 lanes (u32 LE)
//!     22    16  salt (random per file)
//!     38    12  AES-GCM nonce (random per file)
//!     50     …  ciphertext + 16-byte tag; the 50 header bytes are the AAD
//! ```
//!
//! The key is Argon2id(password, salt) with the parameters in the header.
//! Because the header is authenticated, changing any byte of the file makes
//! it fail to open; a wrong password fails the same way.

use std::io::{Read, Write};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use zeroize::Zeroizing;

use super::BackupError;

pub const MAGIC: &[u8; 7] = b"AXOBK1\0";
/// The file format this build writes and the newest it reads.
pub const FORMAT_VERSION: u16 = 1;
const KDF_ARGON2ID: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
pub const HEADER_LEN: usize = MAGIC.len() + 2 + 1 + 4 * 3 + SALT_LEN + NONCE_LEN;

/// Argon2id cost parameters, recorded in each file's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub lanes: u32,
}

impl KdfParams {
    /// What new backups use: 64 MiB, 3 iterations, 1 lane.
    pub const DEFAULT: KdfParams = KdfParams { memory_kib: 64 * 1024, iterations: 3, lanes: 1 };

    /// Bounds accepted when reading, so a damaged or hostile header can't
    /// ask for gigabytes of memory or hours of work.
    fn is_sane(&self) -> bool {
        (8 * 1024..=1024 * 1024).contains(&self.memory_kib) && (1..=16).contains(&self.iterations) && (1..=8).contains(&self.lanes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Header {
    kdf: KdfParams,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
}

impl Header {
    fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        let mut at = 0;
        let mut put = |bytes: &[u8]| {
            out[at..at + bytes.len()].copy_from_slice(bytes);
            at += bytes.len();
        };
        put(MAGIC);
        put(&FORMAT_VERSION.to_le_bytes());
        put(&[KDF_ARGON2ID]);
        put(&self.kdf.memory_kib.to_le_bytes());
        put(&self.kdf.iterations.to_le_bytes());
        put(&self.kdf.lanes.to_le_bytes());
        put(&self.salt);
        put(&self.nonce);
        out
    }

    fn decode(file: &[u8]) -> Result<Header, BackupError> {
        if file.len() < MAGIC.len() || &file[..MAGIC.len()] != MAGIC {
            return Err(BackupError::NotABackup);
        }
        // Too short to hold a header and a tag: cut off or damaged.
        if file.len() < HEADER_LEN + 16 {
            return Err(BackupError::CantOpen);
        }
        let u32_at = |i: usize| u32::from_le_bytes([file[i], file[i + 1], file[i + 2], file[i + 3]]);
        let version = u16::from_le_bytes([file[7], file[8]]);
        if version == 0 {
            return Err(BackupError::NotABackup);
        }
        if version > FORMAT_VERSION {
            return Err(BackupError::NewerVersion);
        }
        if file[9] != KDF_ARGON2ID {
            return Err(BackupError::CantOpen);
        }
        let kdf = KdfParams { memory_kib: u32_at(10), iterations: u32_at(14), lanes: u32_at(18) };
        if !kdf.is_sane() {
            return Err(BackupError::CantOpen);
        }
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&file[22..22 + SALT_LEN]);
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&file[38..38 + NONCE_LEN]);
        Ok(Header { kdf, salt, nonce })
    }
}

fn derive_key(password: &str, salt: &[u8], kdf: KdfParams) -> Result<Zeroizing<[u8; 32]>, BackupError> {
    let params = Params::new(kdf.memory_kib, kdf.iterations, kdf.lanes, Some(32)).map_err(|_| BackupError::CantOpen)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon.hash_password_into(password.as_bytes(), salt, key.as_mut()).map_err(|_| BackupError::CantOpen)?;
    Ok(key)
}

fn random<const N: usize>() -> Result<[u8; N], BackupError> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).map_err(|e| BackupError::Io(format!("This device couldn't provide secure random numbers ({e}).")))?;
    Ok(out)
}

/// Compress and encrypt `plaintext` with a key derived from `password`.
pub fn seal(plaintext: &[u8], password: &str, kdf: KdfParams) -> Result<Vec<u8>, BackupError> {
    let mut compressed = Zeroizing::new(Vec::with_capacity(plaintext.len() / 3));
    {
        let mut enc = DeflateEncoder::new(&mut *compressed, Compression::default());
        enc.write_all(plaintext).map_err(|e| BackupError::Io(format!("Couldn't compress the backup ({e}).")))?;
        enc.finish().map_err(|e| BackupError::Io(format!("Couldn't compress the backup ({e}).")))?;
    }
    let header = Header { kdf, salt: random()?, nonce: random()? }.encode();
    let key = derive_key(password, &header[22..22 + SALT_LEN], kdf)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_ref()));
    let sealed = cipher
        .encrypt(Nonce::from_slice(&header[38..38 + NONCE_LEN]), Payload { msg: compressed.as_slice(), aad: &header })
        .map_err(|_| BackupError::Io("Couldn't encrypt the backup.".into()))?;
    let mut out = Vec::with_capacity(HEADER_LEN + sealed.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Check, decrypt and decompress a whole backup file.
pub fn open(file: &[u8], password: &str) -> Result<Zeroizing<Vec<u8>>, BackupError> {
    let header = Header::decode(file)?;
    let key = derive_key(password, &header.salt, header.kdf)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_ref()));
    let compressed = Zeroizing::new(
        cipher
            .decrypt(Nonce::from_slice(&header.nonce), Payload { msg: &file[HEADER_LEN..], aad: &file[..HEADER_LEN] })
            .map_err(|_| BackupError::CantOpen)?,
    );
    let mut plain = Zeroizing::new(Vec::with_capacity(compressed.len() * 3));
    DeflateDecoder::new(compressed.as_slice()).read_to_end(&mut plain).map_err(|_| BackupError::CantOpen)?;
    Ok(plain)
}

/// Cheap parameters for tests that don't exercise the KDF cost itself.
#[cfg(test)]
pub(crate) const TEST_KDF: KdfParams = KdfParams { memory_kib: 8 * 1024, iterations: 1, lanes: 1 };

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: KdfParams = TEST_KDF;

    #[test]
    fn header_layout_is_stable() {
        let h = Header { kdf: KdfParams::DEFAULT, salt: [7; 16], nonce: [9; 12] };
        let bytes = h.encode();
        assert_eq!(HEADER_LEN, 50);
        assert_eq!(&bytes[..7], b"AXOBK1\0");
        assert_eq!(u16::from_le_bytes([bytes[7], bytes[8]]), 1);
        assert_eq!(bytes[9], 1);
        assert_eq!(u32::from_le_bytes(bytes[10..14].try_into().unwrap()), 65536);
        assert_eq!(u32::from_le_bytes(bytes[14..18].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(bytes[18..22].try_into().unwrap()), 1);
        let mut file = bytes.to_vec();
        file.extend_from_slice(&[0; 16]);
        assert_eq!(Header::decode(&file).unwrap(), h);
    }

    #[test]
    fn seal_and_open_round_trip_with_default_parameters() {
        let text = b"{\"hello\":\"world\"}".repeat(50);
        let file = seal(&text, "correct horse battery", KdfParams::DEFAULT).unwrap();
        assert!(file.len() < text.len(), "compressed before encryption");
        assert_eq!(open(&file, "correct horse battery").unwrap().as_slice(), text.as_slice());
        // Two seals of the same data differ (random salt and nonce).
        let again = seal(&text, "correct horse battery", KdfParams::DEFAULT).unwrap();
        assert_ne!(file, again);
    }

    #[test]
    fn wrong_password_and_tampering_fail_closed() {
        let file = seal(b"secret stuff", "password-1", FAST).unwrap();
        assert!(matches!(open(&file, "password-2"), Err(BackupError::CantOpen)));
        // Salt, nonce, KDF parameters and body are all authenticated.
        for i in [12, 25, 40, HEADER_LEN, file.len() - 1] {
            let mut bad = file.clone();
            bad[i] ^= 0x01;
            assert!(matches!(open(&bad, "password-1"), Err(BackupError::CantOpen)), "byte {i}");
        }
        let mut bad = file.clone();
        bad[0] = b'X';
        assert!(matches!(open(&bad, "password-1"), Err(BackupError::NotABackup)));
        assert!(matches!(open(&file[..30], "password-1"), Err(BackupError::CantOpen)));
        assert!(matches!(open(b"PK\x03\x04 a zip file", "password-1"), Err(BackupError::NotABackup)));
    }

    #[test]
    fn newer_format_version_is_reported_before_decrypting() {
        let mut file = seal(b"x", "password-1", FAST).unwrap();
        file[7..9].copy_from_slice(&2u16.to_le_bytes());
        assert!(matches!(open(&file, "password-1"), Err(BackupError::NewerVersion)));
    }

    /// The AES key schedule inside `Aes256Gcm` is wiped when the cipher is
    /// dropped (its first round keys are the raw key). `ZeroizeOnDrop` is
    /// only implemented with the `aes` crate's own "zeroize" feature, so
    /// this stops building if that feature is ever lost.
    #[test]
    fn aes_key_schedule_is_wiped_on_drop() {
        fn wipes_on_drop<T: zeroize::ZeroizeOnDrop>() {}
        wipes_on_drop::<aes::Aes256>();
        wipes_on_drop::<aes::Aes256Enc>();
    }

    #[test]
    fn absurd_kdf_parameters_are_refused() {
        let mut file = seal(b"x", "password-1", FAST).unwrap();
        file[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(open(&file, "password-1"), Err(BackupError::CantOpen)));
    }
}
