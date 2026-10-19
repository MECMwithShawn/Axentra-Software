//! Saving attachments and messages to the user's Downloads folder.

use std::path::{Path, PathBuf};

/// `$AXENTRA_DOWNLOAD_DIR`, else `XDG_DOWNLOAD_DIR` from `user-dirs.dirs`,
/// else `~/Downloads`.
pub fn downloads_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("AXENTRA_DOWNLOAD_DIR").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return d;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    if let Ok(dirs) = std::fs::read_to_string(config.join("user-dirs.dirs")) {
        for line in dirs.lines() {
            if let Some(v) = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=") {
                let v = v.trim_matches('"').replace("$HOME", &home.to_string_lossy());
                let p = PathBuf::from(v);
                if p.is_absolute() {
                    return p;
                }
            }
        }
    }
    home.join("Downloads")
}

/// A file name safe to create: no path separators or control characters.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c == '/' || c == '\\' || c.is_control() || c == ':' { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').to_string();
    if trimmed.is_empty() { "attachment".into() } else { trimmed }
}

/// `dir/name`, or `dir/name (2).ext`, ... when the name is taken.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let name = safe_file_name(name);
    let first = dir.join(&name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    (2..10_000).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Write `data` into Downloads under `name`; returns the path written.
pub fn save_to_downloads(name: &str, data: &[u8]) -> std::io::Result<PathBuf> {
    let dir = downloads_dir();
    std::fs::create_dir_all(&dir)?;
    let path = unique_path(&dir, name);
    std::fs::write(&path, data)?;
    Ok(path)
}

/// Open a file with the desktop's default app (`xdg-open`), from a private
/// copy under the cache directory (Open on an attachment card).
pub fn open_with_default_app(name: &str, data: &[u8]) -> std::io::Result<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".cache"));
    let dir = cache.join("axentra-outlook").join("open");
    std::fs::create_dir_all(&dir)?;
    let path = unique_path(&dir, name);
    std::fs::write(&path, data)?;
    std::process::Command::new("xdg-open").arg(&path).spawn()?;
    Ok(path)
}

/// CRC-32 (IEEE), as zip files use.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// A zip archive of `files` (name, content), stored without compression
/// (attachments such as PDFs, images and Office files are compressed
/// already). "Download all" saves one of these.
pub fn zip_stored(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let le16 = |v: &mut Vec<u8>, n: u16| v.extend_from_slice(&n.to_le_bytes());
    let le32 = |v: &mut Vec<u8>, n: u32| v.extend_from_slice(&n.to_le_bytes());
    for (name, data) in files {
        // Unique, safe names inside the archive.
        let mut entry = safe_file_name(name);
        let mut n = 2;
        while names.iter().any(|x| x.eq_ignore_ascii_case(&entry)) {
            entry = match safe_file_name(name).rsplit_once('.') {
                Some((stem, ext)) => format!("{stem} ({n}).{ext}"),
                None => format!("{} ({n})", safe_file_name(name)),
            };
            n += 1;
        }
        names.push(entry.clone());
        let crc = crc32(data);
        let offset = out.len() as u32;
        // Local file header; bit 11 marks UTF-8 names.
        le32(&mut out, 0x0403_4b50);
        le16(&mut out, 20);
        le16(&mut out, 1 << 11);
        le16(&mut out, 0);
        le16(&mut out, 0);
        le16(&mut out, 0x21);
        le32(&mut out, crc);
        le32(&mut out, data.len() as u32);
        le32(&mut out, data.len() as u32);
        le16(&mut out, entry.len() as u16);
        le16(&mut out, 0);
        out.extend_from_slice(entry.as_bytes());
        out.extend_from_slice(data);
        // Central directory record.
        le32(&mut central, 0x0201_4b50);
        le16(&mut central, 20);
        le16(&mut central, 20);
        le16(&mut central, 1 << 11);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0x21);
        le32(&mut central, crc);
        le32(&mut central, data.len() as u32);
        le32(&mut central, data.len() as u32);
        le16(&mut central, entry.len() as u16);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le32(&mut central, 0);
        le32(&mut central, offset);
        central.extend_from_slice(entry.as_bytes());
    }
    let central_offset = out.len() as u32;
    let central_len = central.len() as u32;
    out.extend_from_slice(&central);
    // End of central directory.
    le32(&mut out, 0x0605_4b50);
    le16(&mut out, 0);
    le16(&mut out, 0);
    le16(&mut out, names.len() as u16);
    le16(&mut out, names.len() as u16);
    le32(&mut out, central_len);
    le32(&mut out, central_offset);
    le16(&mut out, 0);
    out
}

/// Attachment types blocked for safety (Outlook's list plus Linux
/// executables, docs/spec/mail.md §5.5).
pub fn is_blocked_type(name: &str) -> bool {
    const BLOCKED: &[&str] = &[
        "exe", "bat", "cmd", "com", "vbs", "vbe", "js", "jse", "wsf", "wsh", "msi", "scr", "ps1", "lnk", "hta", "cpl",
        "sh", "run", "desktop", "appimage", "jar", "deb", "rpm",
    ];
    name.rsplit_once('.').is_some_and(|(_, ext)| BLOCKED.contains(&ext.to_ascii_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_uniqueness() {
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name("  "), "attachment");
        let dir = std::env::temp_dir().join(format!("axentra-files-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (2).txt"));
        assert_eq!(unique_path(&dir, "b.txt"), dir.join("b.txt"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(is_blocked_type("setup.EXE"));
        assert!(is_blocked_type("tool.AppImage"));
        assert!(!is_blocked_type("report.pdf"));
    }

    fn u16_at(b: &[u8], i: usize) -> u16 {
        u16::from_le_bytes([b[i], b[i + 1]])
    }

    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
    }

    #[test]
    fn zip_archives_are_well_formed() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        let zip = zip_stored(&[("a.txt".into(), b"hello".to_vec()), ("a.txt".into(), b"again".to_vec()), ("../x.csv".into(), Vec::new())]);
        // End of central directory: 3 entries, directory right after the data.
        let eocd = zip.len() - 22;
        assert_eq!(u32_at(&zip, eocd), 0x0605_4b50);
        assert_eq!(u16_at(&zip, eocd + 10), 3);
        let (cd_len, cd_off) = (u32_at(&zip, eocd + 12) as usize, u32_at(&zip, eocd + 16) as usize);
        assert_eq!(cd_off + cd_len, eocd);
        // Walk the central directory back to each local header and its data.
        let mut i = cd_off;
        let mut seen = Vec::new();
        while i < cd_off + cd_len {
            assert_eq!(u32_at(&zip, i), 0x0201_4b50);
            let size = u32_at(&zip, i + 24) as usize;
            let name_len = u16_at(&zip, i + 28) as usize;
            let local = u32_at(&zip, i + 42) as usize;
            let name = String::from_utf8(zip[i + 46..i + 46 + name_len].to_vec()).unwrap();
            assert_eq!(u32_at(&zip, local), 0x0403_4b50);
            let data_at = local + 30 + u16_at(&zip, local + 26) as usize;
            let data = &zip[data_at..data_at + size];
            assert_eq!(crc32(data), u32_at(&zip, i + 16));
            seen.push((name, String::from_utf8(data.to_vec()).unwrap()));
            i += 46 + name_len;
        }
        assert_eq!(
            seen,
            vec![("a.txt".into(), "hello".into()), ("a (2).txt".into(), "again".into()), ("_x.csv".into(), String::new())]
        );
    }
}
