//! The explorer's file operations and the desktop: file manager, terminal and fonts.
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value as J};

use crate::files::{io_msg, name_of};

/// Case-insensitive order where file2 sorts before file10.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    x.next();
                }
                let mut m = String::new();
                while let Some(c) = y.peek().copied().filter(char::is_ascii_digit) {
                    m.push(c);
                    y.next();
                }
                let (nt, mt) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = nt.len().cmp(&mt.len()).then_with(|| nt.cmp(mt)).then_with(|| n.len().cmp(&m.len()));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(p), Some(q)) => {
                let o = p.to_lowercase().cmp(q.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

pub fn list_dir(path: &Path) -> Result<J, String> {
    let dir = std::fs::read_dir(path).map_err(|e| format!("Couldn't open {}: {}.", name_of(path), io_msg(&e)))?;
    let mut entries: Vec<(bool, String, u64, bool)> = dir
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let link = e.file_type().map(|t| t.is_symlink()).unwrap_or(false);
            let meta = std::fs::metadata(e.path()).ok();
            let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
            (is_dir, name, meta.map(|m| m.len()).unwrap_or(0), link)
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| natural_cmp(&a.1, &b.1)));
    Ok(json!({ "entries": entries.into_iter().map(|(dir, name, size, link)| json!({ "name": name, "dir": dir, "size": size, "link": link })).collect::<Vec<_>>() }))
}

pub fn new_file(path: &Path) -> Result<J, String> {
    std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| format!("Couldn't create {}: {}.", name_of(path), io_msg(&e)))?;
    Ok(json!({ "path": path.display().to_string() }))
}

pub fn new_folder(path: &Path) -> Result<J, String> {
    std::fs::create_dir(path).map_err(|e| format!("Couldn't create {}: {}.", name_of(path), io_msg(&e)))?;
    Ok(json!({ "path": path.display().to_string() }))
}

pub fn rename(from: &Path, to: &Path) -> Result<J, String> {
    if to.symlink_metadata().is_ok() {
        return Err(format!("Couldn't rename {}: {} already exists.", name_of(from), name_of(to)));
    }
    std::fs::rename(from, to).map_err(|e| format!("Couldn't rename {}: {}.", name_of(from), io_msg(&e)))?;
    Ok(json!({ "path": to.display().to_string() }))
}

pub fn trash(path: &Path) -> Result<J, String> {
    if path.symlink_metadata().is_err() {
        return Err(format!("Couldn't move {} to the Trash: it doesn't exist.", name_of(path)));
    }
    let out = Command::new("gio").arg("trash").arg("--").arg(path).stdin(Stdio::null()).output();
    match out {
        Ok(o) if o.status.success() => Ok(json!({})),
        Ok(o) => {
            let msg = String::from_utf8_lossy(&o.stderr);
            let detail = msg.rsplit(": ").next().unwrap_or("").trim().trim_end_matches('.').to_lowercase();
            Err(format!("Couldn't move {} to the Trash{}.", name_of(path), if detail.is_empty() { String::new() } else { format!(": {detail}") }))
        }
        Err(_) => Err("Couldn't move it to the Trash: the gio tool isn't installed.".into()),
    }
}

/// Starts a desktop program without waiting for it; a thread reaps it when it exits.
pub fn spawn(mut cmd: Command) -> bool {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
            true
        }
        Err(_) => false,
    }
}

pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(program)).find(|p| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            p.is_file()
        }
    })
}

fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.display().to_string().bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Shows a file selected in the file manager, or opens a folder.
pub fn reveal(path: &Path) -> Result<J, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("Couldn't show {}: {}.", name_of(path), io_msg(&e)))?;
    if meta.is_file() {
        let shown = Command::new("dbus-send")
            .args(["--session", "--print-reply", "--dest=org.freedesktop.FileManager1", "--type=method_call", "/org/freedesktop/FileManager1", "org.freedesktop.FileManager1.ShowItems"])
            .arg(format!("array:string:{}", file_uri(path)))
            .arg("string:")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if shown {
            return Ok(json!({}));
        }
    }
    let dir = if meta.is_dir() { path } else { path.parent().unwrap_or(path) };
    let mut cmd = Command::new("xdg-open");
    cmd.arg(dir);
    if spawn(cmd) {
        Ok(json!({}))
    } else {
        Err("Couldn't open the file manager: xdg-open isn't installed.".into())
    }
}

/// Opens a terminal in the folder (or the file's folder).
pub fn terminal(path: &Path) -> Result<J, String> {
    let dir = if path.is_dir() { path.to_path_buf() } else { path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.to_path_buf()) };
    if !dir.is_dir() {
        return Err(format!("Couldn't open a terminal: {} doesn't exist.", name_of(&dir)));
    }
    let d = dir.display().to_string();
    let candidates: Vec<(&str, Vec<String>)> = vec![
        ("gnome-terminal", vec![format!("--working-directory={d}")]),
        ("ptyxis", vec![format!("--working-directory={d}")]),
        ("kgx", vec![format!("--working-directory={d}")]),
        ("konsole", vec!["--workdir".into(), d.clone()]),
        ("xfce4-terminal", vec![format!("--working-directory={d}")]),
        ("tilix", vec!["-w".into(), d.clone()]),
        ("alacritty", vec!["--working-directory".into(), d.clone()]),
        ("kitty", vec!["--directory".into(), d.clone()]),
        ("x-terminal-emulator", vec![]),
    ];
    for (program, args) in candidates {
        if let Some(exe) = which(program) {
            let mut cmd = Command::new(exe);
            cmd.args(args).current_dir(&dir);
            if spawn(cmd) {
                return Ok(json!({ "terminal": program }));
            }
        }
    }
    Err("Couldn't find a terminal app to open.".into())
}

/// Monospace font families, for the font picker.
pub fn mono_fonts() -> Vec<String> {
    static FONTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FONTS
        .get_or_init(|| {
            let Ok(out) = Command::new("fc-list").args([":spacing=mono", "family"]).stdin(Stdio::null()).output() else { return Vec::new() };
            let mut list: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| l.split(',').next())
                .map(|f| f.replace('\\', "").trim().to_string())
                .filter(|f| !f.is_empty())
                .collect();
            list.sort_by(|a, b| natural_cmp(a, b));
            list.dedup();
            list
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::tests::temp;

    #[test]
    fn natural_order() {
        let mut v = vec!["file10.txt", "File2.txt", "file1.txt", "b", "A", "file02.txt"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["A", "b", "file1.txt", "File2.txt", "file02.txt", "file10.txt"]);
    }

    #[test]
    fn lists_folders_first() {
        let dir = temp("listdir");
        for f in ["b.txt", "a10.txt", "a9.txt", ".env"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        std::fs::create_dir(dir.join("zeta")).unwrap();
        std::fs::create_dir(dir.join("Alpha")).unwrap();
        let r = list_dir(&dir).unwrap();
        let names: Vec<&str> = r["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["Alpha", "zeta", ".env", "a9.txt", "a10.txt", "b.txt"]);
        assert_eq!(r["entries"][0]["dir"], true);
        assert_eq!(r["entries"][2]["size"], 1);
        assert!(list_dir(&dir.join("nope")).unwrap_err().ends_with("it doesn't exist."));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn create_and_rename() {
        let dir = temp("fsops");
        new_file(&dir.join("a.txt")).unwrap();
        assert!(new_file(&dir.join("a.txt")).unwrap_err().contains("already exists"));
        new_folder(&dir.join("sub")).unwrap();
        assert!(new_folder(&dir.join("sub")).is_err());
        assert!(rename(&dir.join("a.txt"), &dir.join("sub")).unwrap_err().contains("sub already exists"));
        rename(&dir.join("a.txt"), &dir.join("b.txt")).unwrap();
        assert!(dir.join("b.txt").is_file() && !dir.join("a.txt").exists());
        assert!(file_uri(Path::new("/a b/ü.txt")) == "file:///a%20b/%C3%BC.txt");
        std::fs::remove_dir_all(dir).ok();
    }
}
