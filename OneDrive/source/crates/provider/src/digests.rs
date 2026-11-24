//! What a file's content is said to be, and how far that has been checked.

use std::io::Read;

use axentra_onedrive_graph::QuickXorHash;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};

/// A file's size and whichever hashes are known for it.
///
/// Where the values came from matters: a provider's listing *claims* them,
/// [`ContentDigests::of_reader`] *computes* them from bytes on disk. Only
/// computed values prove anything about those bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentDigests {
    pub size: u64,
    /// OneDrive's QuickXorHash, base64. Catches corruption, not tampering:
    /// it is not a cryptographic hash.
    pub qxh: Option<String>,
    /// Lower-case hex.
    pub sha1: Option<String>,
    /// Lower-case hex.
    pub sha256: Option<String>,
}

impl ContentDigests {
    /// Size, QuickXorHash, SHA-1 and SHA-256 of everything `r` yields, in
    /// one pass.
    pub fn of_reader(r: &mut impl Read) -> std::io::Result<Self> {
        let mut qxh = QuickXorHash::new();
        let mut sha1 = Sha1::new();
        let mut sha = Sha256::new();
        let mut size = 0u64;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            qxh.update(&buf[..n]);
            sha1.update(&buf[..n]);
            sha.update(&buf[..n]);
            size += n as u64;
        }
        Ok(Self {
            size,
            qxh: Some(qxh.base64()),
            sha1: Some(hex(&sha1.finalize())),
            sha256: Some(hex(&sha.finalize())),
        })
    }

    pub fn of_bytes(data: &[u8]) -> Self {
        Self::of_reader(&mut &data[..]).expect("reading from memory")
    }

    /// Check these (computed) digests against `expected` (claimed ones).
    /// The sizes must agree and so must every hash both sides have, and at
    /// least one hash must be shared: a size match alone proves nothing.
    pub fn verify_against(&self, expected: &ContentDigests) -> Result<(), String> {
        if self.size != expected.size {
            return Err(format!(
                "size is {} bytes, expected {}",
                self.size, expected.size
            ));
        }
        let pairs = [
            ("QuickXorHash", &self.qxh, &expected.qxh),
            ("SHA-1", &self.sha1, &expected.sha1),
            ("SHA-256", &self.sha256, &expected.sha256),
        ];
        let mut compared = 0;
        for (name, got, want) in pairs {
            if let (Some(got), Some(want)) = (got, want) {
                if !got.eq_ignore_ascii_case(want) {
                    return Err(format!("{name} is {got}, expected {want}"));
                }
                compared += 1;
            }
        }
        if compared == 0 {
            return Err("no hash in common to compare".into());
        }
        Ok(())
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// How far a mirrored file has been verified, in order. A file is recorded
/// at the highest level actually reached, never rounded up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum VerifyLevel {
    /// In the plan. Nothing has been transferred, so nothing is proven.
    L0Planned,
    /// The staged local copy hashes to the source's own QuickXorHash and
    /// size. Proves the local bytes are the version the source described.
    /// QuickXor catches corruption in transit, not deliberate tampering.
    L1SourceVerified,
    /// The destination accepted the upload and reported no failure. Says
    /// nothing about what it stored.
    L2Uploaded,
    /// The digest the destination reports for the file (its claimed SHA-1)
    /// and size match ours. Proves the metadata it recorded, not the bytes
    /// it actually stores.
    L3RemoteClaimed,
    /// The file was downloaded back through the destination, which ran its
    /// own integrity checks, and the SHA-256 of what came back matches the
    /// source's. The only level that proves the stored bytes, and only as
    /// of when the check ran.
    L4RoundTrip,
}

impl VerifyLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            VerifyLevel::L0Planned => "planned",
            VerifyLevel::L1SourceVerified => "source-verified",
            VerifyLevel::L2Uploaded => "uploaded",
            VerifyLevel::L3RemoteClaimed => "remote-claimed",
            VerifyLevel::L4RoundTrip => "round-trip",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_of_known_input() {
        let d = ContentDigests::of_bytes(b"abc");
        assert_eq!(d.size, 3);
        assert_eq!(
            d.sha1.as_deref(),
            Some("a9993e364706816aba3e25717850c26c9cd0d89d")
        );
        assert_eq!(
            d.sha256.as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        let mut q = QuickXorHash::new();
        q.update(b"abc");
        assert_eq!(d.qxh, Some(q.base64()));
    }

    #[test]
    fn verification_needs_a_shared_hash_and_the_same_size() {
        let got = ContentDigests::of_bytes(b"hello");
        assert!(got.verify_against(&got).is_ok());
        let size_only = ContentDigests {
            size: 5,
            ..Default::default()
        };
        assert!(got.verify_against(&size_only).is_err());
        let mut other = ContentDigests::of_bytes(b"hellp");
        assert!(got.verify_against(&other).is_err());
        other.size = 6;
        assert!(got.verify_against(&other).unwrap_err().contains("size"));
        // Hex case doesn't matter (OneDrive's SHA-1 is upper case).
        let mut upper = got.clone();
        upper.sha256 = upper.sha256.map(|s| s.to_ascii_uppercase());
        assert!(got.verify_against(&upper).is_ok());
    }

    #[test]
    fn levels_are_ordered() {
        assert!(VerifyLevel::L0Planned < VerifyLevel::L1SourceVerified);
        assert!(VerifyLevel::L3RemoteClaimed < VerifyLevel::L4RoundTrip);
        assert_eq!(VerifyLevel::L4RoundTrip.as_str(), "round-trip");
    }
}
