use crate::{
    CHUNK, Cancel, Progress, check_cancel,
    device::{self, Device},
    image::{self, Inspection, Iso},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Verify,
    Raw,
    Compatible,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub action: Action,
    pub device: Device,
    pub image_path: PathBuf,
    pub image_sha256: String,
    pub confirmation: String,
}

pub fn authorize(r: &Request) -> Result<()> {
    if r.action != Action::Verify {
        ensure!(
            r.confirmation == r.device.token(),
            "Device-specific erase confirmation is missing"
        );
    }
    ensure!(
        r.image_sha256.len() == 64 && r.image_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "An inspected image hash is required"
    );
    if let Some(reason) = r.device.blocked() {
        anyhow::bail!(reason);
    }
    Ok(())
}
pub fn compare(
    source: &mut dyn Read,
    target: &mut dyn Read,
    len: u64,
    progress: &mut Progress<'_>,
    cancel: &Cancel<'_>,
) -> Result<String> {
    let mut a = vec![0u8; CHUNK];
    let mut b = vec![0u8; CHUNK];
    let mut done = 0;
    let mut hash = Sha256::new();
    while done < len {
        check_cancel(cancel)?;
        let n = (len - done).min(CHUNK as u64) as usize;
        source
            .read_exact(&mut a[..n])
            .context("Short read from image")?;
        target
            .read_exact(&mut b[..n])
            .context("Short read from USB")?;
        if let Some(i) = a[..n].iter().zip(&b[..n]).position(|(a, b)| a != b) {
            anyhow::bail!("USB differs from the image at byte {}", done + i as u64);
        }
        hash.update(&b[..n]);
        done += n as u64;
        progress("Verifying every byte", done, len);
    }
    Ok(hex::encode(hash.finalize()))
}
pub fn write_image(
    source: &mut dyn Read,
    target: &mut dyn Write,
    size: u64,
    sector: usize,
    progress: &mut Progress<'_>,
    cancel: &Cancel<'_>,
) -> Result<()> {
    ensure!([512, 4096].contains(&sector), "Unsupported sector size");
    let mut buf = vec![0u8; CHUNK + sector];
    let mut done = 0;
    while done < size {
        check_cancel(cancel)?;
        let n = (size - done).min(CHUNK as u64) as usize;
        source
            .read_exact(&mut buf[..n])
            .context("Image ended before the expected size")?;
        let padded = n.div_ceil(sector) * sector;
        buf[n..padded].fill(0);
        target.write_all(&buf[..padded])?;
        done += n as u64;
        progress("Writing original image", done, size);
    }
    target.flush()?;
    Ok(())
}
struct SectorReader<'a> {
    file: &'a mut File,
    sector: usize,
}
impl Read for SectorReader<'_> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        if b.len().is_multiple_of(self.sector) {
            return self.file.read(b);
        }
        let mut padded = vec![0; b.len().div_ceil(self.sector) * self.sector];
        let n = self.file.read(&mut padded)?.min(b.len());
        b[..n].copy_from_slice(&padded[..n]);
        Ok(n)
    }
}
fn raw(
    info: &Inspection,
    d: &Device,
    write: bool,
    progress: &mut Progress<'_>,
    cancel: &Cancel<'_>,
) -> Result<Value> {
    let mut source = File::open(&info.path)?;
    let mut usb = device::open_raw(d, write)?;
    if write {
        check_cancel(cancel)?;
        let tail = 1024 * 1024;
        if info.size + tail < d.size {
            usb.file.seek(SeekFrom::Start(d.size - tail))?;
            usb.file.write_all(&vec![0; tail as usize])?;
        }
        usb.file.seek(SeekFrom::Start(0))?;
        write_image(
            &mut source,
            &mut usb.file,
            info.size,
            d.sector as usize,
            progress,
            cancel,
        )?;
        usb.file.sync_all()?;
        usb.file.seek(SeekFrom::Start(0))?;
        source.seek(SeekFrom::Start(0))?;
    }
    let hash = compare(
        &mut source,
        &mut SectorReader {
            file: &mut usb.file,
            sector: d.sector as usize,
        },
        info.size,
        progress,
        cancel,
    )?;
    ensure!(
        hash == info.sha256,
        "Image changed after inspection. No successful verification can be reported."
    );
    Ok(
        json!({"write_integrity":"Every image byte matches","verified_bytes":info.size,"readback_sha256":hash,"firmware_boot":"Not tested"}),
    )
}

pub fn copy_and_verify(
    iso: &Iso,
    info: &Inspection,
    boot: &Path,
    data: &Path,
    progress: &mut Progress<'_>,
    cancel: &Cancel<'_>,
    flush: impl FnOnce() -> Result<()>,
) -> Result<Value> {
    let total = info.content_bytes + info.boot_bytes;
    let mut done = 0;
    let mut manifest: Vec<(PathBuf, u64, String)> = vec![];
    for entry in iso.files.values() {
        image::safe_path(&entry.path)?;
        let roots: Vec<&Path> = if image::boot_asset(&entry.path) {
            vec![data, boot]
        } else {
            vec![data]
        };
        for root in roots {
            check_cancel(cancel)?;
            let dest = root.join(&entry.path);
            std::fs::create_dir_all(dest.parent().context("No parent directory")?)?;
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&dest)?;
            let mut hash = Sha256::new();
            iso.stream(&entry.path, cancel, |b| {
                out.write_all(b)?;
                hash.update(b);
                done += b.len() as u64;
                progress(&format!("Copying {}", entry.path), done, total);
                Ok(())
            })?;
            out.sync_all()?;
            manifest.push((dest, entry.size, hex::encode(hash.finalize())));
        }
    }
    flush()?;
    done = 0;
    let mut buf = vec![0; CHUNK];
    for (path, size, expected) in &manifest {
        check_cancel(cancel)?;
        let mut f = File::open(path)?;
        ensure!(f.metadata()?.len() == *size, "Copied file size mismatch");
        let mut hash = Sha256::new();
        loop {
            check_cancel(cancel)?;
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
            done += n as u64;
            progress("Verifying copied files", done, total);
        }
        ensure!(
            hex::encode(hash.finalize()) == *expected,
            "Read-back checksum mismatch: {}",
            path.display()
        );
    }
    ensure!(
        std::fs::read_to_string(data.join(".disk/casper-uuid"))?.trim() == info.casper_uuid,
        "Live-media UUID changed"
    );
    let mut magic = [0; 4];
    File::open(data.join("casper/filesystem.squashfs"))?.read_exact(&mut magic)?;
    ensure!(&magic == b"hsqs", "Invalid squashfs header");
    Ok(
        json!({"write_integrity":"All copied files verified with SHA-256","verified_files":manifest.len(),"verified_bytes":total,"layout":"GPT / 1 GiB FAT32 EFI / NTFS live data","firmware_boot":"Not tested","secure_boot":"Original EFI binaries preserved; firmware trust/revocation not tested"}),
    )
}

pub fn execute(
    r: &Request,
    event_path: &Path,
    emit: &mut dyn FnMut(Value),
    cancel: &Cancel<'_>,
) -> Result<Value> {
    authorize(r)?;
    ensure!(
        device::admin(),
        "Administrator/root access is required for physical USB access"
    );
    let d = device::revalidate(&r.device, 0)?;
    let _transaction = device::TransactionLock::acquire(&d)?;
    device::assert_source_elsewhere(
        &d,
        &[
            r.image_path.clone(),
            std::env::current_exe()?,
            event_path.to_owned(),
        ],
    )?;
    let mut progress = |phase: &str, done: u64, total: u64| {
        emit(json!({"type":"progress","phase":phase,"done":done,"total":total}))
    };
    let info = image::inspect(&r.image_path, &mut progress, cancel)?;
    ensure!(
        info.sha256 == r.image_sha256,
        "Image changed since selection. Inspect it again."
    );
    let required = if r.action == Action::Compatible {
        ensure!(
            info.compatible,
            "This ISO did not pass the reviewed compatibility profile"
        );
        info.content_bytes + 1024 * 1024 * 1024 + 256 * 1024 * 1024
    } else {
        info.size.div_ceil(d.sector) * d.sector
    };
    let d = device::revalidate(&r.device, required)?;
    check_cancel(cancel)?;
    let result = match r.action {
        Action::Verify => raw(&info, &d, false, &mut progress, cancel)?,
        Action::Raw => raw(&info, &d, true, &mut progress, cancel)?,
        Action::Compatible => {
            let scratch = tempfile::Builder::new()
                .prefix("axentra-mount-")
                .tempdir()?;
            let iso = Iso::open(&info.path)?;
            check_cancel(cancel)?;
            let volumes = device::create_volumes(&d, scratch.path())?;
            let result = copy_and_verify(
                &iso,
                &info,
                &volumes.boot,
                &volumes.data,
                &mut progress,
                cancel,
                || {
                    device::flush(&volumes.boot)?;
                    device::flush(&volumes.data)
                },
            )?;
            volumes.finish()?;
            result
        }
    };
    Ok(
        json!({"application":"Axentra USB","version":env!("CARGO_PKG_VERSION"),"implementation":"Rust","unix_time":SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),"action":r.action,"device":d,"image":info,"result":result}),
    )
}

pub fn worker(request_path: &Path) -> Result<()> {
    let data = std::fs::read(request_path)?;
    ensure!(data.len() < 128 * 1024, "Invalid worker request size");
    let request: Request = serde_json::from_slice(&data)?;
    let parent = request_path.parent().context("No worker directory")?;
    let event_path = parent.join("events.jsonl");
    let cancel_path = parent.join("cancel");
    let mut out = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&event_path)?;
    let mut last = Instant::now();
    let mut emit = |event: Value| {
        if event["type"] == "progress"
            && event["done"] != event["total"]
            && last.elapsed().as_millis() < 150
        {
            return;
        }
        last = Instant::now();
        let _ = writeln!(out, "{event}");
        let _ = out.flush();
    };
    match execute(&request, &event_path, &mut emit, &|| cancel_path.exists()) {
        Ok(report) => {
            emit(json!({"type":"complete","report":report}));
            Ok(())
        }
        Err(e) => {
            emit(json!({"type":"error","message":format!("{e:#}")}));
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn progress(_: &str, _: u64, _: u64) {}
    #[test]
    fn every_byte() {
        let p = b"abcdef";
        assert_eq!(
            compare(
                &mut Cursor::new(p),
                &mut Cursor::new(p),
                6,
                &mut progress,
                &|| false
            )
            .unwrap(),
            image::hash_bytes(p)
        );
    }
    #[test]
    fn precise_corruption() {
        let e = compare(
            &mut Cursor::new(b"abcdef"),
            &mut Cursor::new(b"abcdxf"),
            6,
            &mut progress,
            &|| false,
        )
        .unwrap_err();
        assert!(e.to_string().contains("byte 4"));
    }
    #[test]
    fn short_read() {
        assert!(
            compare(
                &mut Cursor::new(b"abc"),
                &mut Cursor::new(b"ab"),
                3,
                &mut progress,
                &|| false
            )
            .is_err()
        );
    }
    #[test]
    fn cancellation() {
        assert!(
            compare(
                &mut Cursor::new(b"abc"),
                &mut Cursor::new(b"abc"),
                3,
                &mut progress,
                &|| true
            )
            .is_err()
        );
    }
    #[test]
    fn padding() {
        let mut out = vec![];
        write_image(
            &mut Cursor::new(b"hello"),
            &mut out,
            5,
            512,
            &mut progress,
            &|| false,
        )
        .unwrap();
        assert_eq!(out.len(), 512);
        assert_eq!(&out[..5], b"hello");
        assert!(out[5..].iter().all(|b| *b == 0));
    }
    #[test]
    fn cancellation_precedes_write() {
        let mut out = vec![];
        assert!(
            write_image(
                &mut Cursor::new(b"x"),
                &mut out,
                1,
                512,
                &mut progress,
                &|| true
            )
            .is_err()
        );
        assert!(out.is_empty());
    }
    #[test]
    fn no_false_success_on_short_source() {
        let mut out = vec![];
        assert!(
            write_image(
                &mut Cursor::new(b"x"),
                &mut out,
                5,
                512,
                &mut progress,
                &|| false
            )
            .is_err()
        );
        assert!(out.is_empty());
    }
    #[test]
    fn erase_token_required() {
        let r = Request {
            action: Action::Raw,
            device: Device {
                serial: "serial".into(),
                ..Default::default()
            },
            image_path: PathBuf::new(),
            image_sha256: "a".repeat(64),
            confirmation: "wrong".into(),
        };
        assert!(
            authorize(&r)
                .unwrap_err()
                .to_string()
                .contains("confirmation")
        );
    }
    #[test]
    fn unknown_action_rejected() {
        assert!(serde_json::from_str::<Action>("\"shell\"").is_err());
    }
    fn fixture() -> (tempfile::TempDir, Iso, Inspection) {
        use std::collections::BTreeMap;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.iso");
        let mut source = File::create(&path).unwrap();
        let mut files = BTreeMap::new();
        let mut offset = 0;
        for (name, bytes) in [
            (
                "casper/filesystem.squashfs",
                b"hsqs-live-filesystem".as_slice(),
            ),
            ("casper/vmlinuz", b"kernel".as_slice()),
            (".disk/casper-uuid", b"uuid-1".as_slice()),
            ("efi/boot/bootx64.efi", b"efi-loader".as_slice()),
        ] {
            source.write_all(bytes).unwrap();
            files.insert(
                name.into(),
                image::Entry {
                    path: name.into(),
                    size: bytes.len() as u64,
                    extents: vec![(offset, bytes.len() as u64)],
                },
            );
            offset += bytes.len() as u64;
        }
        let info = Inspection {
            path: path.clone(),
            size: offset,
            sha256: String::new(),
            title: "fixture".into(),
            checks: vec![],
            efi_x64: true,
            compatible: true,
            content_bytes: offset,
            boot_bytes: offset - 19,
            largest_file: "casper/filesystem.squashfs".into(),
            largest_size: 19,
            casper_uuid: "uuid-1".into(),
            file_count: 4,
            initrd_compression: vec![],
            firmware_boot: "Not tested".into(),
        };
        (dir, Iso::fixture(path, files), info)
    }
    #[test]
    fn compatible_copy_verifies_every_file() {
        let (dir, iso, info) = fixture();
        let boot = dir.path().join("boot");
        let data = dir.path().join("data");
        std::fs::create_dir(&boot).unwrap();
        std::fs::create_dir(&data).unwrap();
        let report = copy_and_verify(&iso, &info, &boot, &data, &mut progress, &|| false, || {
            Ok(())
        })
        .unwrap();
        assert_eq!(report["verified_files"], 7);
        assert!(!boot.join("casper/filesystem.squashfs").exists());
        assert_eq!(
            std::fs::read(boot.join("casper/vmlinuz")).unwrap(),
            b"kernel"
        );
    }
    #[test]
    fn compatible_corruption_cannot_pass() {
        let (dir, iso, info) = fixture();
        let boot = dir.path().join("boot");
        let data = dir.path().join("data");
        std::fs::create_dir(&boot).unwrap();
        std::fs::create_dir(&data).unwrap();
        let corrupt = data.join("casper/filesystem.squashfs");
        let result = copy_and_verify(&iso, &info, &boot, &data, &mut progress, &|| false, || {
            std::fs::write(corrupt, b"hsqs-bad-filesystem!")?;
            Ok(())
        });
        assert!(result.is_err());
    }
    #[test]
    fn compatible_cancel_before_copy() {
        let (dir, iso, info) = fixture();
        let boot = dir.path().join("boot");
        let data = dir.path().join("data");
        std::fs::create_dir(&boot).unwrap();
        std::fs::create_dir(&data).unwrap();
        assert!(
            copy_and_verify(
                &iso,
                &info,
                &boot,
                &data,
                &mut progress,
                &|| true,
                || Ok(())
            )
            .is_err()
        );
        assert_eq!(std::fs::read_dir(data).unwrap().count(), 0);
    }
}
