//! Keep file-manager thumbnailers from downloading whole folders.
//!
//! Browsing a folder of online-only photos would otherwise make Nautilus's
//! thumbnailers read (and so download) every file. Their reads are refused;
//! once a file is downloaded for real, the thumbnail "failed" marker is
//! cleared so the thumbnail appears.

use std::path::Path;

use md5::{Digest, Md5};

fn comm(pid: u32) -> Option<String> {
    Some(
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()?
            .trim()
            .to_string(),
    )
}

fn parent(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("PPid:"))?
        .trim()
        .parse()
        .ok()
}

/// "program[pid] from parent[pid]", to say who asked for a download.
pub fn describe(pid: u32) -> String {
    let me = comm(pid).unwrap_or_else(|| "unknown".into());
    match parent(pid).and_then(|pp| Some((pp, comm(pp)?))) {
        Some((pp, name)) => format!("{me}[{pid}] from {name}[{pp}]"),
        None => format!("{me}[{pid}]"),
    }
}

/// Whether `pid` is a thumbnailer or desktop indexer.
pub fn is_background_reader(pid: u32) -> bool {
    let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else {
        return false;
    };
    let comm = comm.trim();
    comm.contains("thumb")
        || comm == "tumblerd"
        || comm.starts_with("tracker")
        || comm.starts_with("localsearch")
}

/// GLib's g_filename_to_uri escaping for paths.
fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_encoded_bytes() {
        let keep = b.is_ascii_alphanumeric() || b"!$&'()*+,-./:=@_~".contains(&b);
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn clear_failure(abs_path: &Path) {
    let Some(cache) = dirs::cache_dir() else {
        return;
    };
    let digest = Md5::digest(file_uri(abs_path).as_bytes());
    let name: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let fail_dir = cache.join("thumbnails/fail");
    let Ok(entries) = std::fs::read_dir(&fail_dir) else {
        return;
    };
    for e in entries.flatten() {
        let _ = std::fs::remove_file(e.path().join(format!("{name}.png")));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn uri_escaping_matches_glib() {
        assert_eq!(
            super::file_uri(std::path::Path::new("/home/a/One Drive/x#1;é.jpg")),
            "file:///home/a/One%20Drive/x%231%3B%C3%A9.jpg"
        );
    }
}
