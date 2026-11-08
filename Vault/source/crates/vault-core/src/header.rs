//! Reads the unencrypted KDBX header: which key-derivation function the database uses and how
//! heavy it is ("Argon2id · 256 MB × 6"), for display and for the KDF budget that is checked
//! before any key derivation runs. Touches nothing secret; the header is stored in the clear at
//! the start of every KeePass file, and its SHA-256 is unkeyed, so anyone who can write the file
//! can set these values.

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

const SIG1: u32 = 0x9AA2_D903;
const SIG2: u32 = 0xB54B_FB67;
/// KeePass 1.x (`.kdb`), which keepass also opens.
const SIG2_KDB: u32 = 0xB54B_FB65;
const ARGON2D: [u8; 16] = [
    0xEF, 0x63, 0x6D, 0xDF, 0x8C, 0x29, 0x44, 0x4B, 0x91, 0xF7, 0xA9, 0xA4, 0x03, 0xE3, 0x0A, 0x0C,
];
const ARGON2ID: [u8; 16] = [
    0x9E, 0x29, 0x8B, 0x19, 0x56, 0xDB, 0x47, 0x73, 0xB2, 0x3D, 0xFC, 0x3E, 0xC6, 0xF0, 0xA1, 0xE6,
];
const AES_KDF: [u8; 16] = [
    0xC9, 0xD9, 0xF3, 0x9A, 0x62, 0x8A, 0x44, 0x60, 0xBF, 0x74, 0x0D, 0x08, 0xC1, 0x8A, 0x4F, 0xEA,
];
/// The KDBX 4 id for AES-KDF (KeePassXC writes it); keepass accepts both.
const AES_KDF_KDBX4: [u8; 16] = [
    0x7C, 0x02, 0xBB, 0x82, 0x79, 0xA7, 0x4A, 0xC0, 0x92, 0x7D, 0x11, 0x4A, 0x00, 0x64, 0x82, 0x38,
];
/// Variant dictionary value types (KDBX 4 KDF parameters).
const TYPE_U32: u8 = 0x04;
const TYPE_U64: u8 = 0x05;
const TYPE_BYTES: u8 = 0x42;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kdf {
    Argon2 {
        id: bool,
        memory_bytes: u64,
        iterations: u64,
        parallelism: u32,
    },
    Aes {
        rounds: u64,
    },
}

impl Kdf {
    /// Short human summary, e.g. "Argon2id · 256 MB × 6" or "AES-KDF · 60,000 rounds".
    pub fn summary(&self) -> String {
        match self {
            Kdf::Argon2 {
                id,
                memory_bytes,
                iterations,
                ..
            } => {
                let mb = memory_bytes / (1024 * 1024);
                format!(
                    "{} · {mb} MB × {iterations}",
                    if *id { "Argon2id" } else { "Argon2d" }
                )
            }
            Kdf::Aes { rounds } => format!("AES-KDF · {} rounds", group_thousands(*rounds)),
        }
    }

    /// The summary plus Argon2's lanes, which the budget also limits.
    pub fn details(&self) -> String {
        match self {
            Kdf::Argon2 { parallelism, .. } => {
                let lanes = if *parallelism == 1 { "lane" } else { "lanes" };
                format!("{} · {parallelism} {lanes}", self.summary())
            }
            Kdf::Aes { .. } => self.summary(),
        }
    }

    /// Roughly how much work one key derivation takes, to compare two settings: Argon2's memory
    /// times its passes (more lanes split the same work), and an AES-KDF round counted as 64
    /// bytes of that, which is about what each costs on a current CPU.
    pub fn cost(&self) -> u128 {
        match *self {
            Kdf::Argon2 {
                memory_bytes,
                iterations,
                ..
            } => u128::from(memory_bytes) * u128::from(iterations),
            Kdf::Aes { rounds } => u128::from(rounds) * 64,
        }
    }

    /// The settings a database saved with `config` gets in its header. None for a function this
    /// app doesn't know (keepass may add some).
    pub fn of_config(config: &keepass::config::KdfConfig) -> Option<Kdf> {
        use keepass::config::KdfConfig;
        match *config {
            KdfConfig::Aes { rounds } => Some(Kdf::Aes { rounds }),
            KdfConfig::Argon2 {
                iterations,
                memory,
                parallelism,
                ..
            } => Some(Kdf::Argon2 {
                id: false,
                memory_bytes: memory,
                iterations,
                parallelism,
            }),
            KdfConfig::Argon2id {
                iterations,
                memory,
                parallelism,
                ..
            } => Some(Kdf::Argon2 {
                id: true,
                memory_bytes: memory,
                iterations,
                parallelism,
            }),
            _ => None,
        }
    }
}

/// The most key derivation a database header may ask for before the app runs it. A tampered
/// header can otherwise make an unlock attempt use any amount of memory and time before the
/// password is even checked. Ordinary settings are far below these limits; a database above them
/// opens only after the user explicitly approves its settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfBudget {
    pub argon2_memory_bytes: u64,
    pub argon2_iterations: u64,
    pub argon2_parallelism: u32,
    pub aes_rounds: u64,
}

impl KdfBudget {
    pub const DEFAULT: KdfBudget = KdfBudget {
        argon2_memory_bytes: 1024 * 1024 * 1024,
        argon2_iterations: 100,
        argon2_parallelism: 64,
        aes_rounds: 100_000_000,
    };

    pub fn allows(&self, kdf: &Kdf) -> bool {
        match *kdf {
            Kdf::Argon2 {
                memory_bytes,
                iterations,
                parallelism,
                ..
            } => {
                memory_bytes <= self.argon2_memory_bytes
                    && iterations <= self.argon2_iterations
                    && parallelism <= self.argon2_parallelism
            }
            Kdf::Aes { rounds } => rounds <= self.aes_rounds,
        }
    }

    /// This budget, raised just enough to allow `kdf`: what approving one database's settings
    /// permits, and nothing heavier.
    pub fn allowing(mut self, kdf: &Kdf) -> KdfBudget {
        match *kdf {
            Kdf::Argon2 {
                memory_bytes,
                iterations,
                parallelism,
                ..
            } => {
                self.argon2_memory_bytes = self.argon2_memory_bytes.max(memory_bytes);
                self.argon2_iterations = self.argon2_iterations.max(iterations);
                self.argon2_parallelism = self.argon2_parallelism.max(parallelism);
            }
            Kdf::Aes { rounds } => self.aes_rounds = self.aes_rounds.max(rounds),
        }
        self
    }

    /// e.g. "Argon2: 1 GiB memory, 100 passes, 64 lanes; AES-KDF: 100,000,000 rounds".
    pub fn summary(&self) -> String {
        const MIB: u64 = 1024 * 1024;
        let memory = if self.argon2_memory_bytes.is_multiple_of(1024 * MIB) {
            format!("{} GiB", self.argon2_memory_bytes / (1024 * MIB))
        } else {
            format!("{} MB", self.argon2_memory_bytes / MIB)
        };
        format!(
            "Argon2: {memory} memory, {} passes, {} lanes; AES-KDF: {} rounds",
            self.argon2_iterations,
            self.argon2_parallelism,
            group_thousands(self.aes_rounds)
        )
    }
}

impl Default for KdfBudget {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Why a header's key derivation settings could not be read. Every case refuses the file: a
/// setting that can't be read can't be checked against the budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HeaderError {
    #[error("not a KeePass database (unrecognized file header)")]
    NotKeePass,
    #[error("the database header is damaged")]
    Malformed,
    #[error("the database header lists its key derivation settings more than once")]
    Duplicate,
    #[error("the database uses an unknown key derivation function")]
    UnknownKdf,
}

fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// `len` bytes at `*pos`, advancing past them.
fn take<'a>(b: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], HeaderError> {
    let end = pos.checked_add(len).ok_or(HeaderError::Malformed)?;
    let data = b.get(*pos..end).ok_or(HeaderError::Malformed)?;
    *pos = end;
    Ok(data)
}

/// KDBX4 KDF parameters: a "variant dictionary" of typed key/value pairs.
///
/// Stricter than keepass where the two could read different values: a repeated key is refused
/// (keepass keeps the last copy), and numbers must have exactly their type's width (keepass
/// reads the first bytes of a longer value).
fn parse_variant_dictionary(d: &[u8]) -> Result<Kdf, HeaderError> {
    use HeaderError::Malformed;
    // Version 1.x.
    if u16le(d, 0).ok_or(Malformed)? >> 8 != 1 {
        return Err(Malformed);
    }
    let mut pos = 2;
    let mut seen = HashSet::new();
    let (mut uuid, mut mem, mut iters, mut par, mut rounds) = (None, None, None, None, None);
    loop {
        let ty = take(d, &mut pos, 1)?[0];
        if ty == 0 {
            break;
        }
        let klen = u32le(d, pos).ok_or(Malformed)? as usize;
        pos += 4;
        let key = take(d, &mut pos, klen)?;
        let vlen = u32le(d, pos).ok_or(Malformed)? as usize;
        pos += 4;
        let val = take(d, &mut pos, vlen)?;
        if !seen.insert(key) {
            return Err(HeaderError::Duplicate);
        }
        let number = || -> Result<u64, HeaderError> {
            match (ty, val.len()) {
                (TYPE_U32, 4) => Ok(u32::from_le_bytes(val.try_into().unwrap()).into()),
                (TYPE_U64, 8) => Ok(u64::from_le_bytes(val.try_into().unwrap())),
                _ => Err(Malformed),
            }
        };
        match key {
            b"$UUID" if ty == TYPE_BYTES => uuid = Some(val),
            b"$UUID" => return Err(Malformed),
            b"M" => mem = Some(number()?),
            b"I" => iters = Some(number()?),
            b"P" => par = Some(u32::try_from(number()?).map_err(|_| Malformed)?),
            b"R" => rounds = Some(number()?),
            _ => {}
        }
    }
    let argon2 = |id| -> Result<Kdf, HeaderError> {
        Ok(Kdf::Argon2 {
            id,
            memory_bytes: mem.ok_or(Malformed)?,
            iterations: iters.ok_or(Malformed)?,
            parallelism: par.ok_or(Malformed)?,
        })
    };
    match uuid.and_then(|u| <[u8; 16]>::try_from(u).ok()) {
        Some(ARGON2ID) => argon2(true),
        Some(ARGON2D) => argon2(false),
        Some(AES_KDF | AES_KDF_KDBX4) => Ok(Kdf::Aes {
            rounds: rounds.ok_or(Malformed)?,
        }),
        _ => Err(HeaderError::UnknownKdf),
    }
}

/// The KDF settings of a KeePass 1.x, KDBX 3.1 or KDBX 4.x file, from its first bytes. These are
/// the exact settings keepass will use for the same bytes, or an error; never a guess.
pub fn parse_kdf(b: &[u8]) -> Result<Kdf, HeaderError> {
    use HeaderError::{Malformed, NotKeePass};
    if u32le(b, 0) != Some(SIG1) {
        return Err(NotKeePass);
    }
    match u32le(b, 4) {
        Some(SIG2) => {}
        // A fixed 124-byte header; AES-KDF rounds are its last four bytes.
        Some(SIG2_KDB) => {
            return Ok(Kdf::Aes {
                rounds: u32le(b, 120).ok_or(Malformed)?.into(),
            });
        }
        _ => return Err(NotKeePass),
    }
    let major = u16le(b, 10).ok_or(NotKeePass)?;
    if major != 3 && major != 4 {
        return Err(NotKeePass);
    }
    let mut pos = 12;
    let mut kdf = None;
    loop {
        let id = take(b, &mut pos, 1)?[0];
        let len = if major == 4 {
            u32le(b, pos).ok_or(Malformed)? as usize
        } else {
            u16le(b, pos).ok_or(Malformed)? as usize
        };
        pos += if major == 4 { 4 } else { 2 };
        let data = take(b, &mut pos, len)?;
        let found = match (major, id) {
            (_, 0) => break,
            (4, 11) => parse_variant_dictionary(data)?,
            (3, 6) => Kdf::Aes {
                rounds: u64::from_le_bytes(data.try_into().map_err(|_| Malformed)?),
            },
            _ => continue,
        };
        if kdf.replace(found).is_some() {
            return Err(HeaderError::Duplicate);
        }
    }
    kdf.ok_or(Malformed)
}

/// The (major, minor) format version of a KDBX file: (4, 1) for KDBX 4.1, the only version this
/// app's writer saves. None for anything that isn't KDBX (KeePass 1.x included).
pub fn version(b: &[u8]) -> Option<(u16, u16)> {
    if u32le(b, 0) != Some(SIG1) || u32le(b, 4) != Some(SIG2) {
        return None;
    }
    Some((u16le(b, 10)?, u16le(b, 8)?))
}

/// Parses the KDF from the first bytes of a KDBX 3.1 or 4.x file, for display.
pub fn kdf_from_bytes(b: &[u8]) -> Option<Kdf> {
    parse_kdf(b).ok()
}

/// Reads just the header of a database file (a few KB at most).
pub fn kdf_of(path: &Path) -> Option<Kdf> {
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut buf)
        .ok()?;
    kdf_from_bytes(&buf)
}

/// Test support: `kdbx` (a KDBX 4 file) with the KDF parameter `key` overwritten by `value` of
/// the same width and the header hash recomputed, as anyone who can write the file could do.
#[cfg(test)]
pub(crate) fn patch_kdf_param(kdbx: &[u8], key: &str, value: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut b = kdbx.to_vec();
    let mut pos = 12;
    let mut patched = false;
    loop {
        let id = b[pos];
        let data = pos + 5;
        pos = data + u32le(&b, pos + 1).unwrap() as usize;
        if id == 0 {
            break;
        }
        if id != 11 {
            continue;
        }
        let mut at = data + 2;
        while b[at] != 0 {
            let k = at + 5;
            let klen = u32le(&b, at + 1).unwrap() as usize;
            let v = k + klen + 4;
            let vlen = u32le(&b, k + klen).unwrap() as usize;
            if &b[k..k + klen] == key.as_bytes() {
                assert_eq!(vlen, value.len(), "same width");
                b[v..v + vlen].copy_from_slice(value);
                patched = true;
            }
            at = v + vlen;
        }
    }
    assert!(patched, "no KDF parameter {key}");
    let hash = Sha256::digest(&b[..pos]);
    b[pos..pos + 32].copy_from_slice(&hash);
    b
}

/// Test support: `kdbx` (a KDBX 4 file) relabelled as KDBX 4.`minor`, header hash recomputed.
#[cfg(test)]
pub(crate) fn patch_minor_version(kdbx: &[u8], minor: u16) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut b = kdbx.to_vec();
    b[8..10].copy_from_slice(&minor.to_le_bytes());
    let mut pos = 12;
    loop {
        let id = b[pos];
        pos += 5 + u32le(&b, pos + 1).unwrap() as usize;
        if id == 0 {
            break;
        }
    }
    let hash = Sha256::digest(&b[..pos]);
    b[pos..pos + 32].copy_from_slice(&hash);
    b
}

#[cfg(test)]
mod tests {
    use keepass::config::KdfConfig;
    use keepass::{Database, DatabaseKey};

    use super::*;

    fn saved_with(kdf: KdfConfig) -> Vec<u8> {
        let mut db = Database::new();
        db.config.kdf_config = kdf;
        let mut out = Vec::new();
        db.save(&mut out, DatabaseKey::new().with_password("pw"))
            .unwrap();
        out
    }

    /// A header with the given (id, data) fields, then the end field.
    fn header(sig2: u32, major: u16, fields: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(SIG1.to_le_bytes());
        b.extend(sig2.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(major.to_le_bytes());
        let end = (0, b"\r\n\r\n".to_vec());
        for (id, data) in fields.iter().chain([&end]) {
            b.push(*id);
            if major == 4 {
                b.extend((data.len() as u32).to_le_bytes());
            } else {
                b.extend((data.len() as u16).to_le_bytes());
            }
            b.extend(data);
        }
        b
    }

    /// A KDBX 4.1 header whose KDF parameters are `items`: (type, key, value).
    fn kdbx4(items: &[(u8, &str, Vec<u8>)]) -> Vec<u8> {
        let mut d = 0x0100u16.to_le_bytes().to_vec();
        for (ty, key, val) in items {
            d.push(*ty);
            d.extend((key.len() as u32).to_le_bytes());
            d.extend(key.as_bytes());
            d.extend((val.len() as u32).to_le_bytes());
            d.extend(val);
        }
        d.push(0);
        header(SIG2, 4, &[(2, vec![0; 16]), (11, d)])
    }

    fn u32v(v: u32) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    fn u64v(v: u64) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    fn argon2id(memory: u64, iterations: u64, parallelism: u32) -> Vec<u8> {
        kdbx4(&[
            (TYPE_BYTES, "$UUID", ARGON2ID.to_vec()),
            (TYPE_U64, "M", u64v(memory)),
            (TYPE_U64, "I", u64v(iterations)),
            (TYPE_U32, "P", u32v(parallelism)),
            (TYPE_U32, "V", u32v(0x13)),
            (TYPE_BYTES, "S", vec![0; 32]),
        ])
    }

    fn aes(uuid: [u8; 16], rounds: u64) -> Vec<u8> {
        kdbx4(&[
            (TYPE_BYTES, "$UUID", uuid.to_vec()),
            (TYPE_U64, "R", u64v(rounds)),
            (TYPE_BYTES, "S", vec![0; 32]),
        ])
    }

    #[test]
    fn reads_argon2id_parameters() {
        let bytes = saved_with(KdfConfig::Argon2id {
            iterations: 6,
            memory: 16 * 1024 * 1024,
            parallelism: 2,
            version: argon2::Version::Version13,
        });
        let kdf = kdf_from_bytes(&bytes).unwrap();
        assert_eq!(
            kdf,
            Kdf::Argon2 {
                id: true,
                memory_bytes: 16 * 1024 * 1024,
                iterations: 6,
                parallelism: 2
            }
        );
        assert_eq!(kdf.summary(), "Argon2id · 16 MB × 6");
        assert_eq!(kdf.details(), "Argon2id · 16 MB × 6 · 2 lanes");
        assert_eq!(parse_kdf(&argon2id(16 << 20, 6, 2)), Ok(kdf));
    }

    #[test]
    fn reads_aes_kdf_rounds() {
        let kdf = kdf_from_bytes(&saved_with(KdfConfig::Aes { rounds: 60_000 })).unwrap();
        assert_eq!(kdf.summary(), "AES-KDF · 60,000 rounds");
        // Both AES-KDF ids, the KDBX 3.1 field and the KeePass 1.x header.
        for bytes in [aes(AES_KDF, 60_000), aes(AES_KDF_KDBX4, 60_000)] {
            assert_eq!(parse_kdf(&bytes), Ok(kdf.clone()));
        }
        let kdbx3 = header(SIG2, 3, &[(2, vec![0; 16]), (6, u64v(60_000))]);
        assert_eq!(parse_kdf(&kdbx3), Ok(kdf.clone()));
        let mut kdb = vec![0u8; 124];
        kdb[..4].copy_from_slice(&SIG1.to_le_bytes());
        kdb[4..8].copy_from_slice(&SIG2_KDB.to_le_bytes());
        kdb[120..].copy_from_slice(&60_000u32.to_le_bytes());
        assert_eq!(parse_kdf(&kdb), Ok(kdf));
        assert_eq!(parse_kdf(&kdb[..123]), Err(HeaderError::Malformed));
    }

    #[test]
    fn rejects_non_keepass_data() {
        assert!(kdf_from_bytes(b"not a database").is_none());
        assert!(kdf_from_bytes(&[]).is_none());
        assert_eq!(parse_kdf(b"not a database"), Err(HeaderError::NotKeePass));
        // KDBX 2 pre-release, or an unknown major version.
        assert_eq!(
            parse_kdf(&header(0xB54B_FB66, 4, &[])),
            Err(HeaderError::NotKeePass)
        );
        assert_eq!(
            parse_kdf(&header(SIG2, 5, &[])),
            Err(HeaderError::NotKeePass)
        );
        // Cut off inside the header, or ending without KDF settings.
        let whole = argon2id(1 << 20, 2, 2);
        assert_eq!(parse_kdf(&whole[..40]), Err(HeaderError::Malformed));
        assert_eq!(
            parse_kdf(&header(SIG2, 4, &[(2, vec![0; 16])])),
            Err(HeaderError::Malformed)
        );
    }

    #[test]
    fn budget_limits_each_parameter() {
        const GIB: u64 = 1024 * 1024 * 1024;
        let budget = KdfBudget::default();
        assert_eq!(budget, KdfBudget::DEFAULT);
        for (bytes, allowed) in [
            (argon2id(GIB, 100, 64), true),
            (argon2id(GIB + 1, 100, 64), false),
            (argon2id(64 * GIB, 2, 2), false),
            (argon2id(GIB, 101, 64), false),
            (argon2id(GIB, 100, 65), false),
            (aes(AES_KDF, 100_000_000), true),
            (aes(AES_KDF_KDBX4, 100_000_001), false),
            (header(SIG2, 3, &[(6, u64v(100_000_001))]), false),
            (header(SIG2, 3, &[(6, u64v(u64::MAX))]), false),
        ] {
            let kdf = parse_kdf(&bytes).unwrap();
            assert_eq!(budget.allows(&kdf), allowed, "{}", kdf.details());
        }
        assert_eq!(
            budget.summary(),
            "Argon2: 1 GiB memory, 100 passes, 64 lanes; AES-KDF: 100,000,000 rounds"
        );
    }

    #[test]
    fn approving_a_kdf_allows_it_and_nothing_heavier() {
        let heavy = parse_kdf(&argon2id(2 << 30, 120, 8)).unwrap();
        let approved = KdfBudget::DEFAULT.allowing(&heavy);
        assert!(!KdfBudget::DEFAULT.allows(&heavy));
        assert!(approved.allows(&heavy));
        for heavier in [
            argon2id((2 << 30) + 1, 120, 8),
            argon2id(2 << 30, 121, 8),
            argon2id(2 << 30, 120, 65),
            aes(AES_KDF, 100_000_001),
        ] {
            assert!(!approved.allows(&parse_kdf(&heavier).unwrap()));
        }
    }

    #[test]
    fn repeated_kdf_settings_are_refused() {
        // keepass keeps the last copy, a first-match reader the first: refuse both.
        let twice_in_dictionary = kdbx4(&[
            (TYPE_BYTES, "$UUID", ARGON2ID.to_vec()),
            (TYPE_U64, "M", u64v(1 << 20)),
            (TYPE_U64, "I", u64v(2)),
            (TYPE_U32, "P", u32v(2)),
            (TYPE_U64, "M", u64v(64 << 30)),
        ]);
        let dictionary = |b: &[u8]| -> Vec<u8> {
            // The KDF field of `argon2id`: after the signature, version and 21-byte cipher field.
            let len = u32le(b, 34).unwrap() as usize;
            b[38..38 + len].to_vec()
        };
        let small = dictionary(&argon2id(1 << 20, 2, 2));
        let huge = dictionary(&argon2id(64 << 30, 2, 2));
        let twice_in_header = header(SIG2, 4, &[(11, small), (11, huge)]);
        let twice_in_kdbx3 = header(SIG2, 3, &[(6, u64v(1000)), (6, u64v(u64::MAX))]);
        for bytes in [twice_in_dictionary, twice_in_header, twice_in_kdbx3] {
            assert_eq!(parse_kdf(&bytes), Err(HeaderError::Duplicate));
        }
    }

    #[test]
    fn unknown_kdf_fails_closed() {
        for bytes in [
            aes([0x11; 16], 1000),
            kdbx4(&[(TYPE_BYTES, "$UUID", ARGON2ID[..15].to_vec())]),
            kdbx4(&[(TYPE_U64, "R", u64v(1000))]),
        ] {
            assert_eq!(parse_kdf(&bytes), Err(HeaderError::UnknownKdf));
        }
    }

    #[test]
    fn numbers_need_their_exact_type_and_width() {
        let with_p = |ty: u8, value: Vec<u8>| {
            kdbx4(&[
                (TYPE_BYTES, "$UUID", ARGON2D.to_vec()),
                (TYPE_U64, "M", u64v(1 << 20)),
                (TYPE_U64, "I", u64v(2)),
                (ty, "P", value),
            ])
        };
        assert_eq!(
            parse_kdf(&with_p(TYPE_U64, u64v(4))),
            Ok(Kdf::Argon2 {
                id: false,
                memory_bytes: 1 << 20,
                iterations: 2,
                parallelism: 4
            })
        );
        for bad in [
            // 2^32 + 1 lanes would wrap to 1 in an unchecked conversion.
            with_p(TYPE_U64, u64v((1 << 32) + 1)),
            with_p(TYPE_U32, u64v(4)),
            with_p(TYPE_U64, vec![4, 0, 0, 0]),
            with_p(0x0C, u32v(4)),
            kdbx4(&[(TYPE_U64, "$UUID", u64v(0))]),
            // Twelve bytes, of which keepass would read the first eight.
            kdbx4(&[
                (TYPE_BYTES, "$UUID", AES_KDF.to_vec()),
                (TYPE_U64, "R", [u64v(1000), u32v(1)].concat()),
            ]),
        ] {
            assert_eq!(parse_kdf(&bad), Err(HeaderError::Malformed));
        }
    }

    #[test]
    fn version_and_cost() {
        let saved = saved_with(KdfConfig::Aes { rounds: 1000 });
        assert_eq!(version(&saved), Some((4, 1)));
        assert_eq!(version(&patch_minor_version(&saved, 0)), Some((4, 0)));
        assert_eq!(version(&header(SIG2, 3, &[(6, u64v(1000))])), Some((3, 1)));
        assert_eq!(version(b"not a database"), None);
        let mut kdb = vec![0u8; 124];
        kdb[..4].copy_from_slice(&SIG1.to_le_bytes());
        kdb[4..8].copy_from_slice(&SIG2_KDB.to_le_bytes());
        assert_eq!(version(&kdb), None);

        let argon2 = |memory_bytes, iterations, parallelism| Kdf::Argon2 {
            id: true,
            memory_bytes,
            iterations,
            parallelism,
        };
        assert_eq!(argon2(64 << 20, 2, 2).cost(), (64 << 21) as u128);
        assert_eq!(argon2(64 << 20, 2, 2).cost(), argon2(64 << 20, 2, 8).cost());
        assert_eq!(Kdf::Aes { rounds: 1000 }.cost(), 64_000);
        assert_eq!(
            argon2(u64::MAX, u64::MAX, 1).cost(),
            u128::from(u64::MAX).pow(2)
        );

        for config in [
            KdfConfig::Aes { rounds: 1000 },
            KdfConfig::Argon2 {
                iterations: 3,
                memory: 1 << 20,
                parallelism: 2,
                version: argon2::Version::Version13,
            },
            KdfConfig::Argon2id {
                iterations: 2,
                memory: 1 << 20,
                parallelism: 1,
                version: argon2::Version::Version13,
            },
        ] {
            assert_eq!(
                Kdf::of_config(&config),
                kdf_from_bytes(&saved_with(config.clone()))
            );
        }
    }

    #[test]
    fn patched_parameters_keep_a_valid_header_hash() {
        let original = saved_with(KdfConfig::Argon2id {
            iterations: 2,
            memory: 1 << 20,
            parallelism: 1,
            version: argon2::Version::Version13,
        });
        let patched = patch_kdf_param(&original, "I", &u64v(3));
        assert_eq!(
            parse_kdf(&patched),
            Ok(Kdf::Argon2 {
                id: true,
                memory_bytes: 1 << 20,
                iterations: 3,
                parallelism: 1
            })
        );
        // keepass accepts the header (hash intact) and fails only on the derived key.
        let err = Database::parse(&patched, DatabaseKey::new().with_password("pw")).unwrap_err();
        assert!(
            matches!(
                err,
                keepass::error::DatabaseOpenError::Key(
                    keepass::error::DatabaseKeyError::IncorrectKey
                )
            ),
            "rejected before key derivation: {err}"
        );
    }
}
