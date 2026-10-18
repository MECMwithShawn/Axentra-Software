use crate::{CHUNK, Cancel, Progress, check_cancel};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const CASPER: &str = "6cfed5189eabf839164da8f51784c5724b74f0f9867fa4676f45ff35227a9ae7";
const HELPERS: &str = "e29ece94d09e2da5f8338d12a4096d8790ddffdc40d89c26a0258056ffbb62f7";
fn le32(b: &[u8], n: usize) -> u32 {
    u32::from_le_bytes(b[n..n + 4].try_into().unwrap())
}
pub fn hash_bytes(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Check {
    pub label: String,
    pub status: String,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Inspection {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
    pub title: String,
    pub checks: Vec<Check>,
    pub efi_x64: bool,
    pub compatible: bool,
    pub content_bytes: u64,
    pub boot_bytes: u64,
    pub largest_file: String,
    pub largest_size: u64,
    pub casper_uuid: String,
    pub file_count: usize,
    pub initrd_compression: Vec<String>,
    pub firmware_boot: String,
}
impl Inspection {
    fn check(&mut self, name: &str, status: &str, detail: impl Into<String>) {
        self.checks.push(Check {
            label: name.into(),
            status: status.into(),
            detail: detail.into(),
        });
    }
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub extents: Vec<(u64, u64)>,
}
pub struct Iso {
    pub path: PathBuf,
    pub files: BTreeMap<String, Entry>,
    len: u64,
    links: Vec<(String, String)>,
}

pub fn safe_path(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && !name.starts_with('/'),
        "Unsafe image path"
    );
    for part in name.split('/') {
        ensure!(
            !part.is_empty() && part != "." && part != "..",
            "Unsafe image path: {name}"
        );
        ensure!(
            !part.ends_with(['.', ' '])
                && !part.chars().any(|c| c < ' ' || "\\:<>\"|?*".contains(c)),
            "Unrepresentable image path: {name}"
        );
        let base = part.split('.').next().unwrap().to_ascii_uppercase();
        ensure!(
            !["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
                && !(base.len() == 4
                    && (base.starts_with("COM") || base.starts_with("LPT"))
                    && matches!(base.as_bytes()[3], b'1'..=b'9')),
            "Reserved image filename"
        );
    }
    Ok(())
}

impl Iso {
    #[cfg(test)]
    pub(crate) fn fixture(path: PathBuf, files: BTreeMap<String, Entry>) -> Self {
        let len = std::fs::metadata(&path).unwrap().len();
        Self {
            path,
            files,
            len,
            links: vec![],
        }
    }
    pub fn open(path: &Path) -> Result<Self> {
        let mut f = File::open(path)?;
        let len = f.metadata()?.len();
        let pvd = read_at(&mut f, 16 * 2048, 2048, len)?;
        ensure!(
            &pvd[1..6] == b"CD001" && pvd[0] == 1,
            "Not an ISO9660 image"
        );
        ensure!(
            u16::from_le_bytes(pvd[128..130].try_into()?) == 2048,
            "Unsupported ISO block size"
        );
        let mut iso = Self {
            path: path.to_owned(),
            files: BTreeMap::new(),
            len,
            links: vec![],
        };
        iso.walk(
            &mut f,
            u64::from(le32(&pvd, 158)) * 2048,
            u64::from(le32(&pvd, 166)),
            "",
            0,
            &mut HashSet::new(),
        )?;
        iso.expand_links()?;
        Ok(iso)
    }
    fn walk(
        &mut self,
        f: &mut File,
        offset: u64,
        length: u64,
        parent: &str,
        depth: usize,
        visited: &mut HashSet<u64>,
    ) -> Result<()> {
        ensure!(
            depth < 40 && self.files.len() < 100_000 && length <= 32 * 1024 * 1024,
            "ISO directory limits exceeded"
        );
        ensure!(visited.insert(offset), "Cyclic ISO directory");
        let data = read_at(f, offset, length as usize, self.len)?;
        let mut p = 0usize;
        let mut continuation: Option<String> = None;
        while p < data.len() {
            let n = data[p] as usize;
            if n == 0 {
                p = (p / 2048 + 1) * 2048;
                continue;
            }
            ensure!(
                n >= 34 && p + n <= data.len(),
                "Truncated ISO directory record"
            );
            let r = &data[p..p + n];
            p += n;
            let namelen = r[32] as usize;
            ensure!(33 + namelen <= r.len(), "Truncated ISO filename");
            let native = &r[33..33 + namelen];
            if native == [0] || native == [1] {
                continue;
            }
            let su = 33 + namelen + usize::from(namelen.is_multiple_of(2));
            let (rrname, link) = if su <= r.len() {
                susp(f, &r[su..], self.len, 0)?
            } else {
                (None, None)
            };
            let name = rrname.unwrap_or_else(|| {
                String::from_utf8_lossy(native)
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('.')
                    .to_lowercase()
            });
            let path = if parent.is_empty() {
                name
            } else {
                format!("{parent}/{name}")
            };
            safe_path(&path)?;
            let start = (u64::from(le32(r, 2)) + u64::from(r[1])) * 2048;
            let bytes = u64::from(le32(r, 10));
            ensure!(
                start.checked_add(bytes).is_some_and(|end| end <= self.len),
                "ISO extent exceeds image size"
            );
            if let Some(target) = link {
                self.links.push((path, target));
                continue;
            }
            if r[25] & 2 != 0 {
                self.walk(f, start, bytes, &path, depth + 1, visited)?;
            } else {
                let key = path.to_lowercase();
                if let Some(entry) = self.files.get_mut(&key) {
                    ensure!(
                        continuation.as_ref() == Some(&key),
                        "Duplicate/colliding ISO path: {path}"
                    );
                    entry.extents.push((start, bytes));
                    entry.size = entry
                        .size
                        .checked_add(bytes)
                        .context("File size overflow")?;
                } else {
                    ensure!(continuation.is_none(), "Broken ISO multi-extent sequence");
                    self.files.insert(
                        key.clone(),
                        Entry {
                            path,
                            size: bytes,
                            extents: vec![(start, bytes)],
                        },
                    );
                }
                continuation = if r[25] & 128 != 0 { Some(key) } else { None };
            }
        }
        ensure!(continuation.is_none(), "Incomplete multi-extent ISO file");
        Ok(())
    }
    fn expand_links(&mut self) -> Result<()> {
        for (link, target) in &self.links {
            ensure!(!target.starts_with('/'), "Absolute ISO symlink unsupported");
            let mut parts: Vec<&str> = link.split('/').collect();
            parts.pop();
            for p in target.split('/') {
                match p {
                    "." | "" => {}
                    ".." => {
                        ensure!(parts.pop().is_some(), "Symlink escapes ISO");
                    }
                    _ => parts.push(p),
                }
            }
            let resolved = parts.join("/");
            safe_path(&resolved)?;
            let key = resolved.to_lowercase();
            let source: Vec<_> = self
                .files
                .iter()
                .filter(|(k, _)| **k == key || k.starts_with(&(key.clone() + "/")))
                .map(|(_, v)| v.clone())
                .collect();
            ensure!(
                !source.is_empty() && source.len() + self.files.len() < 100_000,
                "Unresolved ISO symlink: {link}"
            );
            for mut entry in source {
                entry.path = format!("{}{}", link, &entry.path[resolved.len()..]);
                ensure!(
                    !self.files.contains_key(&entry.path.to_lowercase()),
                    "Symlink path collision"
                );
                self.files.insert(entry.path.to_lowercase(), entry);
            }
        }
        Ok(())
    }
    pub fn stream(
        &self,
        name: &str,
        cancel: &Cancel<'_>,
        mut consume: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        let entry = self
            .files
            .get(&name.to_lowercase())
            .context("File missing from image")?;
        let mut source = File::open(&self.path)?;
        let mut buf = vec![0u8; CHUNK];
        for &(offset, size) in &entry.extents {
            source.seek(SeekFrom::Start(offset))?;
            let mut remaining = size;
            while remaining > 0 {
                check_cancel(cancel)?;
                let amount = remaining.min(CHUNK as u64) as usize;
                source
                    .read_exact(&mut buf[..amount])
                    .context("Incomplete ISO extent")?;
                consume(&buf[..amount])?;
                remaining -= amount as u64;
            }
        }
        Ok(())
    }
    pub fn read(&self, name: &str, limit: u64) -> Result<Vec<u8>> {
        let Some(entry) = self.files.get(&name.to_lowercase()) else {
            return Ok(vec![]);
        };
        ensure!(entry.size <= limit, "Inspection file exceeds memory limit");
        let mut result = Vec::with_capacity(entry.size as usize);
        self.stream(name, &|| false, |b| {
            result.extend_from_slice(b);
            Ok(())
        })?;
        Ok(result)
    }
}

fn read_at(f: &mut File, offset: u64, size: usize, total: u64) -> Result<Vec<u8>> {
    ensure!(
        offset.checked_add(size as u64).is_some_and(|n| n <= total),
        "Archive extent out of bounds"
    );
    f.seek(SeekFrom::Start(offset))?;
    let mut b = vec![0; size];
    f.read_exact(&mut b)?;
    Ok(b)
}
fn susp(
    f: &mut File,
    bytes: &[u8],
    total: u64,
    depth: u8,
) -> Result<(Option<String>, Option<String>)> {
    ensure!(depth < 5, "Too many SUSP continuation records");
    let mut p = 0;
    let mut name = String::new();
    let mut link = String::new();
    let mut has_link = false;
    while p + 4 <= bytes.len() {
        let n = bytes[p + 2] as usize;
        if n < 4 {
            break;
        }
        ensure!(p + n <= bytes.len(), "Truncated SUSP entry");
        let r = &bytes[p..p + n];
        p += n;
        match &r[..2] {
            b"NM" if n >= 5 && r[4] & 6 == 0 => name.push_str(std::str::from_utf8(&r[5..])?),
            b"CE" if n >= 28 => {
                let offset = u64::from(le32(r, 4)) * 2048 + u64::from(le32(r, 12));
                let len = le32(r, 20) as usize;
                ensure!(len <= 1024 * 1024, "SUSP continuation too large");
                let more = read_at(f, offset, len, total)?;
                let (a, b) = susp(f, &more, total, depth + 1)?;
                if let Some(a) = a {
                    name.push_str(&a);
                }
                if let Some(b) = b {
                    link.push_str(&b);
                    has_link = true;
                }
            }
            b"SL" if n >= 5 => {
                has_link = true;
                let mut q = 5;
                while q + 2 <= n {
                    let flags = r[q];
                    let len = r[q + 1] as usize;
                    q += 2;
                    ensure!(q + len <= n, "Truncated symlink");
                    ensure!(
                        flags & 1 == 0,
                        "Split symlink components need a separately reviewed profile"
                    );
                    if !link.is_empty() && !link.ends_with('/') {
                        link.push('/');
                    }
                    match flags & !1 {
                        2 => link.push('.'),
                        4 => link.push_str(".."),
                        8 => link.push('/'),
                        0 => link.push_str(std::str::from_utf8(&r[q..q + len])?),
                        _ => bail!("Unsupported symlink component"),
                    }
                    q += len;
                }
            }
            _ => {}
        }
    }
    Ok(((!name.is_empty()).then_some(name), has_link.then_some(link)))
}

pub fn hash_file(path: &Path, progress: &mut Progress<'_>, cancel: &Cancel<'_>) -> Result<String> {
    let mut f = File::open(path)?;
    let total = f.metadata()?.len();
    let mut done = 0;
    let mut buf = vec![0; CHUNK];
    let mut hash = Sha256::new();
    loop {
        check_cancel(cancel)?;
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        done += n as u64;
        progress("Hashing image", done, total);
    }
    Ok(hex::encode(hash.finalize()))
}

#[derive(Default)]
pub struct Initrd {
    pub names: HashSet<String>,
    pub texts: BTreeMap<String, String>,
    pub compression: Vec<String>,
}
fn cpio(reader: &mut dyn Read, result: &mut Initrd) -> Result<()> {
    let mut consumed = 0u64;
    for _ in 0..200_000 {
        let mut h = [0u8; 110];
        reader.read_exact(&mut h)?;
        ensure!(
            &h[..6] == b"070701" || &h[..6] == b"070702",
            "Unsupported CPIO header"
        );
        let size = u64::from_str_radix(std::str::from_utf8(&h[54..62])?, 16)?;
        let ns = usize::from_str_radix(std::str::from_utf8(&h[94..102])?, 16)?;
        ensure!(
            ns > 0 && ns <= 4096 && size <= 512 * 1024 * 1024,
            "CPIO entry exceeds limits"
        );
        consumed += 110 + ns as u64 + size + 8;
        ensure!(
            consumed <= 1024 * 1024 * 1024,
            "Initramfs exceeds inspection budget"
        );
        let mut name = vec![0; ns];
        reader.read_exact(&mut name)?;
        let name = String::from_utf8(name[..ns - 1].to_vec())?
            .trim_start_matches("./")
            .to_string();
        let mut padding = [0u8; 3];
        reader.read_exact(&mut padding[..(4 - (110 + ns) % 4) % 4])?;
        let wanted = [
            "scripts/casper",
            "scripts/casper-helpers",
            "conf/uuid.conf",
            "conf/conf.d/default-boot-to-casper.conf",
        ]
        .contains(&name.as_str())
            || name.ends_with("/modules.builtin");
        if wanted && size < 4 * 1024 * 1024 {
            let mut data = vec![0; size as usize];
            reader.read_exact(&mut data)?;
            result
                .texts
                .insert(name.clone(), String::from_utf8_lossy(&data).into_owned());
        } else {
            let copied = std::io::copy(&mut reader.take(size), &mut std::io::sink())?;
            ensure!(copied == size, "Truncated CPIO data");
        }
        reader.read_exact(&mut padding[..(4 - size as usize % 4) % 4])?;
        if name == "TRAILER!!!" {
            return Ok(());
        }
        result.names.insert(name);
    }
    bail!("Too many initramfs entries")
}
pub fn inspect_initrd(data: &[u8]) -> Result<Initrd> {
    let mut result = Initrd::default();
    let mut cursor = Cursor::new(data);
    while (cursor.position() as usize) < data.len() {
        while (cursor.position() as usize) < data.len() && data[cursor.position() as usize] == 0 {
            cursor.set_position(cursor.position() + 1);
        }
        let p = cursor.position() as usize;
        if p == data.len() {
            break;
        }
        if data[p..].starts_with(b"07070") {
            result.compression.push("cpio".into());
            cpio(&mut cursor, &mut result)?;
        } else if data[p..].starts_with(&[0x1f, 0x8b]) {
            result.compression.push("gzip".into());
            cpio(&mut flate2::read::GzDecoder::new(&data[p..]), &mut result)?;
            break;
        } else {
            bail!("This initramfs compression is not supported by the reviewed profile");
        }
    }
    Ok(result)
}

pub fn boot_asset(path: &str) -> bool {
    let p = path.to_lowercase();
    ["efi/", "boot/", ".disk/"].iter().any(|x| p.starts_with(x))
        || ["casper/vmlinuz", "casper/initrd.zstd"].contains(&p.as_str())
}
pub fn inspect(
    path: &Path,
    progress: &mut Progress<'_>,
    cancel: &Cancel<'_>,
) -> Result<Inspection> {
    let path = path.canonicalize()?;
    let before = std::fs::metadata(&path)?;
    ensure!(
        before.is_file() && before.len() >= 512,
        "Select an uncompressed ISO or disk image"
    );
    let sha256 = hash_file(&path, progress, cancel)?;
    let mut r = Inspection {
        title: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path: path.clone(),
        size: before.len(),
        sha256,
        checks: vec![],
        efi_x64: false,
        compatible: false,
        content_bytes: 0,
        boot_bytes: 0,
        largest_file: String::new(),
        largest_size: 0,
        casper_uuid: String::new(),
        file_count: 0,
        initrd_compression: vec![],
        firmware_boot: "Not tested".into(),
    };
    r.check(
        "Image integrity",
        "info",
        "SHA-256 calculated locally. Compare with the publisher checksum for authenticity.",
    );
    let mut f = File::open(&path)?;
    f.seek(SeekFrom::Start(32769))?;
    let mut magic = [0; 5];
    if f.read_exact(&mut magic).is_err() || &magic != b"CD001" {
        r.check(
            "Image format",
            "warning",
            "Raw disk image. Bootability is not established.",
        );
        return Ok(r);
    }
    progress("Inspecting the boot chain", 0, 0);
    let iso = Iso::open(&path)?;
    r.file_count = iso.files.len();
    for e in iso.files.values() {
        r.content_bytes += e.size;
        if boot_asset(&e.path) {
            r.boot_bytes += e.size;
        }
        if e.size > r.largest_size {
            r.largest_size = e.size;
            r.largest_file = e.path.clone();
        }
    }
    let title = iso.read(".disk/info", 1024 * 1024)?;
    if !title.is_empty() {
        r.title = String::from_utf8_lossy(&title).trim().into();
    }
    let boot = iso.read("efi/boot/bootx64.efi", 16 * 1024 * 1024)?;
    if boot.len() > 64 && &boot[..2] == b"MZ" {
        let p = le32(&boot, 60) as usize;
        if p + 94 < boot.len() && &boot[p..p + 4] == b"PE\0\0" {
            r.efi_x64 = u16::from_le_bytes(boot[p + 4..p + 6].try_into()?) == 0x8664
                && u16::from_le_bytes(boot[p + 92..p + 94].try_into()?) == 10;
        }
    }
    r.check(
        "UEFI bootloader",
        if r.efi_x64 { "pass" } else { "warning" },
        if r.efi_x64 {
            "x64 removable-media EFI loader found."
        } else {
            "x64 EFI loader not confirmed."
        },
    );
    r.check(
        "FAT32 file limit",
        if r.largest_size > u32::MAX as u64 {
            "warning"
        } else {
            "pass"
        },
        format!(
            "{} is {} bytes. FAT32 allows at most 4,294,967,295 bytes per file.",
            r.largest_file, r.largest_size
        ),
    );
    let needed = [
        "casper/filesystem.squashfs",
        "casper/vmlinuz",
        "casper/initrd.zstd",
        "boot/grub/grub.cfg",
        "efi/boot/grubx64.efi",
        ".disk/casper-uuid",
    ];
    if needed.iter().all(|p| iso.files.contains_key(*p)) {
        match inspect_initrd(&iso.read("casper/initrd.zstd", 128 * 1024 * 1024)?) {
            Ok(init) => {
                let text = |p: &str| init.texts.get(p).map(String::as_str).unwrap_or("");
                let known = hash_bytes(text("scripts/casper").as_bytes()) == CASPER
                    && hash_bytes(text("scripts/casper-helpers").as_bytes()) == HELPERS;
                r.casper_uuid = text("conf/uuid.conf").trim().into();
                let uuid = !r.casper_uuid.is_empty()
                    && r.casper_uuid
                        == String::from_utf8_lossy(&iso.read(".disk/casper-uuid", 4096)?).trim();
                let ntfs = init.names.contains("usr/bin/ntfs-3g")
                    && (init.names.contains("usr/sbin/mount.ntfs")
                        || init.names.contains("sbin/mount.ntfs"));
                let fuse = init.names.iter().any(|n| n.contains("/fuse.ko"))
                    || init.texts.iter().any(|(k, v)| {
                        k.ends_with("/modules.builtin") && v.contains("kernel/fs/fuse/fuse.ko")
                    });
                let grub = String::from_utf8(iso.read("boot/grub/grub.cfg", 1024 * 1024)?)?;
                let boot_mode =
                    text("conf/conf.d/default-boot-to-casper.conf").contains("BOOT=casper");
                r.compatible = known
                    && uuid
                    && ntfs
                    && fuse
                    && boot_mode
                    && r.efi_x64
                    && grub.contains("/casper/vmlinuz")
                    && grub.contains("/casper/initrd.zstd")
                    && r.boot_bytes < 900 * 1024 * 1024;
                r.check("Live filesystem discovery",if known&&uuid{"pass"}else{"warning"},"Casper revision and live-media UUID checked against the reviewed Zorin profile.");
                r.check(
                    "NTFS startup support",
                    if ntfs && fuse { "pass" } else { "warning" },
                    "NTFS tools and FUSE support checked inside the actual initramfs.",
                );
                r.initrd_compression = init.compression;
            }
            Err(e) => r.check("Startup environment", "warning", e.to_string()),
        }
    } else {
        r.check(
            "Compatibility profile",
            "warning",
            "Required Zorin/Casper files are missing. Original-image mode remains available.",
        );
    }
    r.check(
        "Firmware boot test",
        "info",
        "Not performed. Image checks do not establish actual hardware boot or Secure Boot trust.",
    );
    let after = std::fs::metadata(path)?;
    ensure!(
        before.len() == after.len() && before.modified()? == after.modified()?,
        "Image changed during inspection"
    );
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsafe_paths() {
        for p in [
            "../escape",
            "/etc/passwd",
            "a/../b",
            "C:/x",
            "a\\b",
            "a/NUL.txt",
            "a/file:stream",
        ] {
            assert!(safe_path(p).is_err(), "{p}");
        }
    }
    #[test]
    fn hidden_path() {
        assert!(safe_path(".disk/casper-uuid").is_ok());
    }
    #[test]
    fn multi_extent() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"HEADabcPADdefTAIL").unwrap();
        let iso = Iso {
            path: f.path().to_owned(),
            files: BTreeMap::from([(
                "large".into(),
                Entry {
                    path: "large".into(),
                    size: 6,
                    extents: vec![(4, 3), (10, 3)],
                },
            )]),
            len: 16,
            links: vec![],
        };
        assert_eq!(iso.read("large", 10).unwrap(), b"abcdef");
    }
    #[test]
    fn truncated_initrd() {
        assert!(inspect_initrd(b"070701000").is_err());
    }
    #[test]
    fn rejects_unsupported_compression() {
        assert!(inspect_initrd(&[0x28, 0xb5, 0x2f, 0xfd]).is_err());
    }
    #[test]
    fn boot_selection() {
        assert!(boot_asset("EFI/BOOT/BOOTX64.EFI"));
        assert!(boot_asset(".disk/casper-uuid"));
        assert!(!boot_asset("casper/filesystem.squashfs"));
    }
}
