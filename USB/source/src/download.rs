//! Unprivileged HTTPS download, cancellation, and mandatory saved-file verification.
use crate::{Cancel, Progress};
use anyhow::{Context, Result, ensure};
use reqwest::{Client, Url};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub const ZORIN_URL: &str = "https://mirror.umd.edu/zorin/18/Zorin-OS-18.1-Core-64-bit.iso";
pub const ZORIN_SHA256: &str = "44649b97bd307fc4c8529205d098ebbf98575a1e1ba2ee7d7005b697af1721d5";
pub const ZORIN_FILENAME: &str = "Zorin-OS-18.1-Core-64-bit.iso";
pub const CHECKSUM_PAGE: &str =
    "https://help.zorin.com/docs/getting-started/check-the-integrity-of-your-copy-of-zorin-os/";

#[derive(Debug, Serialize)]
pub struct Download {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

fn check_cancel(cancel: &Cancel<'_>) -> Result<()> {
    ensure!(
        !cancel(),
        "Download cancelled. No completed image was saved."
    );
    Ok(())
}

fn validate_url(value: &str, https_only: bool) -> Result<Url> {
    let url = Url::parse(value).context("Enter a valid HTTPS download URL")?;
    ensure!(
        url.scheme() == "https" || (!https_only && url.scheme() == "http"),
        "Downloads require HTTPS"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "URL credentials are not supported"
    );
    ensure!(
        url.host_str().is_some() && url.fragment().is_none(),
        "URL must have a host and no fragment"
    );
    Ok(url)
}

pub fn valid_checksum(hash: &str) -> bool {
    let hash = hash.trim();
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

// Poll cancellation during DNS, connection, TLS, headers, and each body read.
async fn wait<F: Future>(future: F, cancel: &Cancel<'_>, stall: Duration) -> Result<F::Output> {
    tokio::pin!(future);
    let deadline = tokio::time::sleep(stall);
    tokio::pin!(deadline);
    loop {
        check_cancel(cancel)?;
        tokio::select! {
            result = &mut future => return Ok(result),
            _ = &mut deadline => anyhow::bail!("Download stalled; retry when the connection is available"),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {},
        }
    }
}

pub fn download(
    url: &str,
    hash: &str,
    destination: &Path,
    progress: &mut Progress<'_>,
    cancel: &Cancel<'_>,
) -> Result<Download> {
    transfer(
        url,
        hash,
        destination,
        progress,
        cancel,
        true,
        Duration::from_secs(30),
    )
}

fn transfer(
    url: &str,
    hash: &str,
    destination: &Path,
    progress: &mut Progress<'_>,
    cancel: &Cancel<'_>,
    https_only: bool,
    stall: Duration,
) -> Result<Download> {
    check_cancel(cancel)?;
    ensure!(
        valid_checksum(hash),
        "A publisher SHA-256 (64 hexadecimal characters) is required"
    );
    let url = validate_url(url, https_only)?;
    ensure!(
        destination
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("iso")),
        "Choose an .iso destination"
    );
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let destination = parent.join(
        destination
            .file_name()
            .context("Missing destination filename")?,
    );
    match std::fs::symlink_metadata(&destination) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
        Ok(_) => anyhow::bail!("Destination already exists; choose a new filename"),
    }
    let client = Client::builder()
        .https_only(https_only)
        .referer(false)
        .user_agent(concat!("AxentraUSB/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("Too many download redirects")
            } else if validate_url(attempt.url().as_str(), https_only).is_err() {
                attempt.error("Unsafe download redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()?;
    let mut file = tempfile::Builder::new()
        .prefix(".axentra-download-")
        .suffix(".part")
        .tempfile_in(&parent)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let bytes = runtime.block_on(async {
        progress("Connecting", 0, 0);
        let mut response = wait(
            client.get(url).header("Accept-Encoding", "identity").send(),
            cancel,
            stall,
        )
        .await?
        .map_err(|e| e.without_url())?;
        ensure!(
            response.status() == reqwest::StatusCode::OK,
            "Download server returned HTTP {}",
            response.status().as_u16()
        );
        let total = response.content_length().unwrap_or(0);
        let mut done = 0u64;
        while let Some(chunk) = wait(response.chunk(), cancel, stall)
            .await?
            .map_err(|e| e.without_url())?
        {
            check_cancel(cancel)?;
            file.write_all(&chunk)
                .context("Could not save download (check free disk space)")?;
            done += chunk.len() as u64;
            progress("Downloading", done, total);
        }
        ensure!(
            done > 0 && (total == 0 || done == total),
            "Download was empty or incomplete"
        );
        Ok::<u64, anyhow::Error>(done)
    })?;
    check_cancel(cancel)?;
    file.as_file().sync_all()?;
    // Hash the saved file, not only the network stream.
    let mut saved = file.reopen()?;
    let mut digest = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut checked = 0;
    progress("Verifying SHA-256", 0, bytes);
    loop {
        check_cancel(cancel)?;
        let count = saved.read(&mut buf)?;
        if count == 0 {
            break;
        }
        digest.update(&buf[..count]);
        checked += count as u64;
        progress("Verifying SHA-256", checked, bytes);
    }
    let actual = hex::encode(digest.finalize());
    ensure!(
        checked == bytes && actual.eq_ignore_ascii_case(hash.trim()),
        "SHA-256 mismatch. Download discarded; check the publisher checksum and retry."
    );
    check_cancel(cancel)?;
    drop(saved);
    // Atomic no-clobber publication; a concurrent file is never overwritten.
    file.persist_noclobber(&destination)
        .map_err(|e| e.error)
        .context("Could not publish verified ISO; destination may already exist")?;
    Ok(Download {
        path: destination,
        bytes,
        sha256: actual,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
    };
    fn server(response: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/image.iso", listener.local_addr().unwrap());
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(&response);
        });
        (url, thread)
    }
    fn request(response: &[u8], expected: &str) -> (Result<Download>, tempfile::TempDir) {
        let (url, thread) = server(response.to_vec());
        let dir = tempfile::tempdir().unwrap();
        let result = transfer(
            &url,
            expected,
            &dir.path().join("image.iso"),
            &mut |_, _, _| {},
            &|| false,
            false,
            Duration::from_secs(2),
        );
        thread.join().unwrap();
        (result, dir)
    }
    fn hash(body: &[u8]) -> String {
        hex::encode(Sha256::digest(body))
    }
    #[test]
    fn verified_bytes_published_and_progress_reported() {
        let (url, thread) = server(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\niso".to_vec());
        let dir = tempfile::tempdir().unwrap();
        let mut phases = vec![];
        let result = transfer(
            &url,
            &hash(b"iso"),
            &dir.path().join("image.iso"),
            &mut |p, n, t| phases.push((p.to_owned(), n, t)),
            &|| false,
            false,
            Duration::from_secs(2),
        )
        .unwrap();
        thread.join().unwrap();
        assert_eq!(std::fs::read(result.path).unwrap(), b"iso");
        assert!(
            phases
                .iter()
                .any(|(p, n, t)| p == "Downloading" && *n == 3 && *t == 3)
        );
        assert!(
            phases
                .iter()
                .any(|(p, n, t)| p == "Verifying SHA-256" && *n == *t && *t == 3)
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn chunked_unknown_length() {
        let (result, _) = request(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\niso\r\n0\r\n\r\n",
            &hash(b"iso"),
        );
        assert_eq!(result.unwrap().bytes, 3);
    }
    #[test]
    fn mismatch_truncation_and_http_error_leave_no_file() {
        for response in [
            &b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nbad"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\niso"[..],
            &b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"[..],
        ] {
            let (result, dir) = request(response, &hash(b"iso"));
            assert!(result.is_err());
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }
    #[test]
    fn production_rejects_http_credentials_and_missing_checksum() {
        for url in [
            "http://example.com/a.iso",
            "file:///a.iso",
            "https://user:secret@example.com/a.iso",
            "https://example.com/a#fragment",
        ] {
            assert!(validate_url(url, true).is_err());
        }
        for h in ["", "abc", &"g".repeat(64)] {
            assert!(!valid_checksum(h));
        }
    }
    #[test]
    fn existing_destination_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.iso");
        std::fs::write(&path, b"original").unwrap();
        assert!(
            download(
                "https://example.com/a.iso",
                &hash(b"iso"),
                &path,
                &mut |_, _, _| {},
                &|| false
            )
            .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), b"original");
    }
    #[test]
    fn cancellation_during_transfer_and_verification_cleans_up() {
        for stage in ["Downloading", "Verifying SHA-256"] {
            let (url, thread) = server(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\niso".to_vec());
            let dir = tempfile::tempdir().unwrap();
            let cancel = AtomicBool::new(false);
            let result = transfer(
                &url,
                &hash(b"iso"),
                &dir.path().join("image.iso"),
                &mut |p, _, _| {
                    if p == stage {
                        cancel.store(true, Ordering::Relaxed);
                    }
                },
                &|| cancel.load(Ordering::Relaxed),
                false,
                Duration::from_secs(2),
            );
            thread.join().unwrap();
            assert!(result.unwrap_err().to_string().contains("cancelled"));
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }
    #[test]
    fn cancellation_while_waiting_for_headers_is_prompt() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let server = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            flag.store(true, Ordering::Relaxed);
            thread::sleep(Duration::from_millis(400));
        });
        let dir = tempfile::tempdir().unwrap();
        let start = std::time::Instant::now();
        let result = transfer(
            &url,
            &hash(b"iso"),
            &dir.path().join("image.iso"),
            &mut |_, _, _| {},
            &|| cancelled.load(Ordering::Relaxed),
            false,
            Duration::from_secs(5),
        );
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        server.join().unwrap();
    }
    #[test]
    fn redirect_is_followed_and_verified() {
        let (url, destination) =
            server(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\niso".to_vec());
        let (result, _) = request(
            format!("HTTP/1.1 302 Found\r\nLocation: {url}\r\nContent-Length: 0\r\n\r\n")
                .as_bytes(),
            &hash(b"iso"),
        );
        destination.join().unwrap();
        assert!(result.is_ok());
    }
    #[test]
    fn concurrent_destination_preserved() {
        let (url, thread) = server(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\niso".to_vec());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.iso");
        let result = transfer(
            &url,
            &hash(b"iso"),
            &path,
            &mut |phase, _, _| {
                if phase == "Verifying SHA-256" {
                    std::fs::write(&path, b"keep me").unwrap();
                }
            },
            &|| false,
            false,
            Duration::from_secs(2),
        );
        thread.join().unwrap();
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn stalled_body_times_out_and_removes_partial() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n")
                .unwrap();
            thread::sleep(Duration::from_millis(500));
        });
        let dir = tempfile::tempdir().unwrap();
        let result = transfer(
            &url,
            &hash(b"iso"),
            &dir.path().join("image.iso"),
            &mut |_, _, _| {},
            &|| false,
            false,
            Duration::from_millis(150),
        );
        thread.join().unwrap();
        assert!(result.unwrap_err().to_string().contains("stalled"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
