//! Image preparation is separate from device writing. No block-device APIs here.
use crate::{Cancel, Progress, download, image};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
struct Request {
    source: PathBuf,
    sha256: String,
}

#[derive(Debug, Serialize)]
pub struct Prepared {
    pub source_sha256: String,
    pub path: PathBuf,
    pub sha256: String,
}

pub fn startup_prepared(info: &image::Inspection) -> bool {
    info.checks
        .iter()
        .any(|c| c.label == "Intel IMSM/RST startup" && c.status == "info")
}

pub fn prepare(
    source: &Path,
    checksum: &str,
    destination: &Path,
    progress: &mut Progress<'_>,
    cancel: &Cancel<'_>,
) -> Result<Prepared> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (source, checksum, destination, progress, cancel);
        anyhow::bail!("RAID image preparation currently requires Linux");
    }
    #[cfg(target_os = "linux")]
    {
        use std::{
            io::{BufRead, BufReader},
            process::{Command, Stdio},
            sync::mpsc,
            time::Duration,
        };
        ensure!(
            unsafe { libc::geteuid() } != 0,
            "Run the desktop app as your normal user"
        );
        ensure!(
            download::valid_checksum(checksum),
            "Enter the publisher SHA-256 before preparing RAID support"
        );
        let source = source.canonicalize()?;
        ensure!(source.is_file(), "Select a regular source ISO");
        let parent = destination
            .parent()
            .context("Choose an output folder")?
            .canonicalize()?;
        let destination = parent.join(destination.file_name().context("Choose an ISO filename")?);
        ensure!(
            destination
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("iso")),
            "Output must be an .iso file"
        );
        ensure!(
            std::fs::symlink_metadata(&destination)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "Output already exists or cannot be accessed"
        );
        crate::check_cancel(cancel)?;
        let job = tempfile::Builder::new()
            .prefix("axentra-raid-job-")
            .tempdir()?;
        let request = job.path().join("request.json");
        std::fs::write(
            &request,
            serde_json::to_vec(&Request {
                source,
                sha256: checksum.trim().into(),
            })?,
        )?;
        let file = tempfile::Builder::new()
            .prefix(".axentra-raid-")
            .suffix(".iso")
            .tempfile_in(parent)?;
        progress(
            "Authenticate to install build tools and prepare the RAID image",
            0,
            0,
        );
        let mut child = Command::new("pkexec")
            .arg(std::env::current_exe()?)
            .arg("--raid-worker")
            .arg(&request)
            .stdin(Stdio::null())
            .stdout(file.as_file().try_clone()?)
            .stderr(Stdio::piped())
            .spawn()
            .context("Could not launch administrator authentication (pkexec)")?;
        let stderr = child.stderr.take().unwrap();
        let (tx, rx) = mpsc::sync_channel(64);
        let logs = std::thread::spawn(move || {
            let mut recent = std::collections::VecDeque::new();
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(stage) = line.strip_prefix("AXENTRA: ") {
                    let _ = tx.try_send(stage.to_string());
                }
                if !line.trim().is_empty() {
                    recent.push_back(line.chars().take(500).collect::<String>());
                    if recent.len() > 16 {
                        recent.pop_front();
                    }
                }
            }
            recent.into_iter().collect::<Vec<_>>().join("\n")
        });
        let status = loop {
            if cancel() {
                let _ = std::fs::write(job.path().join("cancel"), b"cancel");
            }
            for message in rx.try_iter() {
                progress(&message, 0, 0);
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let detail = logs.join().unwrap_or_default();
        crate::check_cancel(cancel)?;
        ensure!(
            status.success(),
            "RAID preparation failed or authentication was cancelled: {detail}"
        );
        file.as_file().sync_all()?;
        let info = image::inspect(file.path(), progress, cancel)?;
        ensure!(
            info.efi_x64 && startup_prepared(&info),
            "Prepared image failed EFI/IMSM inspection; no completed ISO saved"
        );
        crate::check_cancel(cancel)?;
        file.persist_noclobber(&destination).map_err(|e| e.error)?;
        Ok(Prepared {
            source_sha256: checksum.trim().to_lowercase(),
            path: destination,
            sha256: info.sha256,
        })
    }
}

pub fn worker(request: &Path) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = request;
        anyhow::bail!("Linux required");
    }
    #[cfg(target_os = "linux")]
    {
        use std::{os::unix::fs::PermissionsExt, process::Command};
        ensure!(
            unsafe { libc::geteuid() } == 0,
            "Administrator authentication is required"
        );
        let bundle = tempfile::Builder::new()
            .prefix("axentra-raid-tools-")
            .tempdir_in("/var/tmp")?;
        for (name, content) in [
            ("prepare.py", include_str!("../raid/prepare.py")),
            ("INSTALL-RAID.txt", include_str!("../raid/INSTALL-RAID.txt")),
            ("build-iso.sh", include_str!("../raid/build-iso.sh")),
            ("axentra_raid.py", include_str!("../raid/axentra_raid.py")),
            ("axentra-imsm", include_str!("../raid/axentra-imsm")),
            ("zz-axentra-imsm", include_str!("../raid/zz-axentra-imsm")),
        ] {
            let path = bundle.path().join(name);
            std::fs::write(&path, content)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
        let status = Command::new("/usr/bin/python3")
            .arg(bundle.path().join("prepare.py"))
            .arg(request)
            .status()?;
        ensure!(status.success(), "RAID build did not complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_or_missing_startup_evidence_is_not_prepared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.iso");
        std::fs::write(&path, [0u8; 512]).unwrap();
        let mut info = image::inspect(&path, &mut |_, _, _| {}, &|| false).unwrap();
        assert!(!startup_prepared(&info));
        info.checks.push(image::Check {
            label: "Intel IMSM/RST startup".into(),
            status: "warning".into(),
            detail: "Missing".into(),
        });
        assert!(!startup_prepared(&info));
        info.checks.last_mut().unwrap().status = "info".into();
        assert!(startup_prepared(&info));
    }
}
