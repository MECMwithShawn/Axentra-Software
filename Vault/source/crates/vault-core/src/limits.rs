//! Size caps for files read whole into memory. A database, key file or passkey file is chosen by
//! the user or sits next to the database (often in a synced folder), so a huge or endless file
//! must fail with an error instead of exhausting memory.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use zeroize::Zeroizing;

/// Key files: KeePass accepts any file and hashes it, so the cap is generous.
pub const KEY_FILE_MAX: u64 = 16 * 1024 * 1024;
/// The `.axvault` passkey file: at most a few kilobytes even with many keys.
pub const SIDECAR_MAX: u64 = 64 * 1024;
/// KeePass databases, attachments included.
pub const DATABASE_MAX: u64 = 512 * 1024 * 1024;

/// Reads the whole file, failing with `FileTooLarge` (and nothing kept) if it holds more than
/// `max` bytes. The buffer is sized from the file length up front, so secret contents are not
/// left behind in freed, regrown allocations, and zeroized on drop.
pub fn read_capped(path: &Path, max: u64) -> io::Result<Zeroizing<Vec<u8>>> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    if len > max {
        return Err(too_large(max));
    }
    // One spare byte, so reading to the end needs no regrowth and a file that grew past `max`
    // since the length check is still caught.
    let mut buf = Zeroizing::new(Vec::with_capacity(len as usize + 1));
    file.take(max + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > max {
        return Err(too_large(max));
    }
    Ok(buf)
}

fn too_large(max: u64) -> io::Error {
    let size = if max >= 1024 * 1024 {
        format!("{} MiB", max / (1024 * 1024))
    } else {
        format!("{} KiB", max / 1024)
    };
    io::Error::new(
        io::ErrorKind::FileTooLarge,
        format!("the file is larger than {size}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random_bytes;

    fn temp_file(contents: &[u8]) -> std::path::PathBuf {
        let mut n = [0u8; 8];
        random_bytes(&mut n);
        let path = std::env::temp_dir().join(format!("axvault-cap-{}.bin", hex::encode(n)));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn reads_up_to_the_cap_and_rejects_one_byte_more() {
        let path = temp_file(&[7u8; 100]);
        assert_eq!(&read_capped(&path, 100).unwrap()[..], &[7u8; 100][..]);
        let err = read_capped(&path, 99).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::FileTooLarge);
        assert!(err.to_string().contains("larger than"), "{err}");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn empty_and_missing_files() {
        let path = temp_file(&[]);
        assert!(read_capped(&path, SIDECAR_MAX).unwrap().is_empty());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            read_capped(&path, SIDECAR_MAX).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn caps_are_the_documented_sizes() {
        assert_eq!(KEY_FILE_MAX, 16 << 20);
        assert_eq!(SIDECAR_MAX, 64 << 10);
        assert_eq!(DATABASE_MAX, 512 << 20);
        assert_eq!(
            too_large(SIDECAR_MAX).to_string(),
            "the file is larger than 64 KiB"
        );
        assert_eq!(
            too_large(DATABASE_MAX).to_string(),
            "the file is larger than 512 MiB"
        );
    }
}
