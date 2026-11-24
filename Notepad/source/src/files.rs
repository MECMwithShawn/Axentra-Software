//! Files on disk: encodings, line endings, atomic saves, recent lists and the session backup.
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value as J};

/// The largest file Axentra Notepad opens.
pub const MAX_OPEN: u64 = 100 * 1024 * 1024;

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    base.join("axentra/notepad")
}

pub fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// An io::Error as the end of a sentence: "permission denied", "it doesn't exist".
pub fn io_msg(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "it doesn't exist".into(),
        io::ErrorKind::PermissionDenied => "permission denied".into(),
        io::ErrorKind::AlreadyExists => "something with that name already exists".into(),
        _ => {
            let s = e.to_string();
            let s = s.split(" (os error").next().unwrap_or(&s).to_string();
            let mut c = s.chars();
            match c.next() {
                Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
                None => "an unknown problem".into(),
            }
        }
    }
}

pub fn err(msg: impl Into<String>) -> J {
    json!({ "ok": false, "error": msg.into() })
}

pub fn mtime_ms(meta: &std::fs::Metadata) -> f64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// True when this user can't write to the path (read-only bit, ownership or a read-only mount).
pub fn readonly(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
        // SAFETY: a valid NUL-terminated path; access() only reads it.
        unsafe { libc::access(c.as_ptr(), libc::W_OK) != 0 }
    }
    #[cfg(not(unix))]
    {
        std::fs::metadata(path).map(|m| m.permissions().readonly()).unwrap_or(false)
    }
}

/// Writes next to the target and renames over it, so a failed save never leaves half a file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().ok_or("that isn't a file name")?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.axentra-{}", std::process::id()));
    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
        if let Ok(meta) = std::fs::metadata(path) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        std::fs::rename(&tmp, path)
    })();
    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            // A folder we can't create files in can still hold a file we may write (shared dirs).
            if e.kind() == io::ErrorKind::PermissionDenied && path.is_file() && !readonly(path) {
                let mut f = File::create(path).map_err(|e| io_msg(&e))?;
                f.write_all(bytes).and_then(|_| f.sync_all()).map_err(|e| io_msg(&e))?;
                return Ok(());
            }
            Err(io_msg(&e))
        }
    }
}

// ---------------------------------------------------------------- encodings

pub const ENCODINGS: &[(&str, &str)] = &[
    ("utf-8", "UTF-8"),
    ("utf-8-bom", "UTF-8 with BOM"),
    ("utf-16le", "UTF-16 LE"),
    ("utf-16be", "UTF-16 BE"),
    ("windows-1252", "Western (Windows 1252)"),
    ("iso-8859-15", "Western (ISO 8859-15)"),
    ("windows-1250", "Central European (Windows 1250)"),
    ("iso-8859-2", "Central European (ISO 8859-2)"),
    ("windows-1251", "Cyrillic (Windows 1251)"),
    ("koi8-r", "Cyrillic (KOI8-R)"),
    ("ibm866", "Cyrillic (DOS 866)"),
    ("windows-1253", "Greek (Windows 1253)"),
    ("windows-1254", "Turkish (Windows 1254)"),
    ("windows-1255", "Hebrew (Windows 1255)"),
    ("windows-1256", "Arabic (Windows 1256)"),
    ("windows-1257", "Baltic (Windows 1257)"),
    ("windows-1258", "Vietnamese (Windows 1258)"),
    ("windows-874", "Thai (Windows 874)"),
    ("shift_jis", "Japanese (Shift JIS)"),
    ("euc-jp", "Japanese (EUC-JP)"),
    ("gbk", "Chinese Simplified (GBK)"),
    ("gb18030", "Chinese Simplified (GB18030)"),
    ("big5", "Chinese Traditional (Big5)"),
    ("euc-kr", "Korean (EUC-KR)"),
    ("macintosh", "Western (Mac Roman)"),
];

pub fn encoding_name(id: &str) -> &'static str {
    ENCODINGS.iter().find(|e| e.0 == id).map(|e| e.1).unwrap_or("UTF-8")
}

pub fn encodings_json() -> J {
    J::Array(ENCODINGS.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect())
}

fn legacy(id: &str) -> Option<&'static encoding_rs::Encoding> {
    if matches!(id, "utf-8" | "utf-8-bom" | "utf-16le" | "utf-16be") || !ENCODINGS.iter().any(|e| e.0 == id) {
        return None;
    }
    encoding_rs::Encoding::for_label(id.as_bytes())
}

pub struct Decoded {
    pub text: String,
    pub encoding: &'static str,
    pub lossy: bool,
}

fn utf16(bytes: &[u8], le: bool) -> (String, bool) {
    let units = bytes.chunks(2).map(|c| {
        let pair = [c[0], *c.get(1).unwrap_or(&0)];
        if le { u16::from_le_bytes(pair) } else { u16::from_be_bytes(pair) }
    });
    let mut lossy = bytes.len() % 2 == 1;
    let text = char::decode_utf16(units)
        .map(|r| {
            r.unwrap_or_else(|_| {
                lossy = true;
                '\u{fffd}'
            })
        })
        .collect();
    (text, lossy)
}

/// BOM-less UTF-16: mostly-ASCII text has a zero in every other byte.
fn sniff_utf16(bytes: &[u8]) -> Option<bool> {
    let head = &bytes[..bytes.len().min(4096) & !1];
    let pairs = head.len() / 2;
    if pairs < 2 {
        return None;
    }
    let even = head.iter().step_by(2).filter(|b| **b == 0).count();
    let odd = head.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
    if odd * 10 >= pairs * 4 && even * 20 < pairs {
        Some(true)
    } else if even * 10 >= pairs * 4 && odd * 20 < pairs {
        Some(false)
    } else {
        None
    }
}

/// Decodes file bytes, detecting the encoding unless `forced` names one.
pub fn decode(bytes: &[u8], forced: Option<&str>) -> Decoded {
    let forced = forced.filter(|f| *f != "auto" && ENCODINGS.iter().any(|e| e.0 == *f));
    let id: &'static str = match forced {
        Some(f) => ENCODINGS.iter().find(|e| e.0 == f).unwrap().0,
        None => {
            if bytes.starts_with(b"\xEF\xBB\xBF") {
                "utf-8-bom"
            } else if bytes.starts_with(b"\xFF\xFE") {
                "utf-16le"
            } else if bytes.starts_with(b"\xFE\xFF") {
                "utf-16be"
            } else if let Some(le) = bytes.contains(&0).then(|| sniff_utf16(bytes)).flatten() {
                if le { "utf-16le" } else { "utf-16be" }
            } else if std::str::from_utf8(bytes).is_ok() {
                "utf-8"
            } else {
                "windows-1252"
            }
        }
    };
    let (text, lossy) = match id {
        "utf-8" | "utf-8-bom" => {
            let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
            match String::from_utf8_lossy(b) {
                std::borrow::Cow::Borrowed(s) => (s.to_string(), false),
                std::borrow::Cow::Owned(s) => (s, true),
            }
        }
        "utf-16le" => utf16(bytes.strip_prefix(b"\xFF\xFE").unwrap_or(bytes), true),
        "utf-16be" => utf16(bytes.strip_prefix(b"\xFE\xFF").unwrap_or(bytes), false),
        _ => {
            let enc = legacy(id).unwrap_or(encoding_rs::WINDOWS_1252);
            let (cow, had_errors) = enc.decode_without_bom_handling(bytes);
            (cow.into_owned(), had_errors)
        }
    };
    Decoded { text, encoding: id, lossy }
}

/// Encodes text; Err carries the first character the encoding can't represent.
pub fn encode(text: &str, id: &str) -> Result<Vec<u8>, char> {
    match id {
        "utf-8-bom" => Ok([b"\xEF\xBB\xBF".as_slice(), text.as_bytes()].concat()),
        "utf-16le" | "utf-16be" => {
            let le = id == "utf-16le";
            let mut out = if le { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
            for u in text.encode_utf16() {
                out.extend_from_slice(&if le { u.to_le_bytes() } else { u.to_be_bytes() });
            }
            Ok(out)
        }
        _ => match legacy(id) {
            None => Ok(text.as_bytes().to_vec()),
            Some(enc) => {
                let (bytes, _, unmappable) = enc.encode(text);
                if !unmappable {
                    return Ok(bytes.into_owned());
                }
                let mut buf = [0u8; 4];
                Err(text.chars().find(|c| enc.encode(c.encode_utf8(&mut buf)).2).unwrap_or('\u{fffd}'))
            }
        },
    }
}

// ---------------------------------------------------------------- line endings

pub struct Eol {
    pub eol: &'static str,
    pub detected: bool,
    pub mixed: bool,
}

pub fn detect_eol(text: &str) -> Eol {
    let b = text.as_bytes();
    let (mut crlf, mut lf, mut cr) = (0usize, 0usize, 0usize);
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 1;
            }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    let kinds = [crlf, lf, cr].iter().filter(|n| **n > 0).count();
    let eol = if crlf == 0 && lf == 0 && cr == 0 {
        "lf"
    } else if crlf >= lf && crlf >= cr {
        "crlf"
    } else if lf >= cr {
        "lf"
    } else {
        "cr"
    };
    Eol { eol, detected: kinds > 0, mixed: kinds > 1 }
}

/// Every line ending becomes "\n".
pub fn normalize_eol(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn apply_eol(text: &str, eol: &str) -> String {
    match eol {
        "crlf" => text.replace('\n', "\r\n"),
        "cr" => text.replace('\n', "\r"),
        _ => text.to_string(),
    }
}

// ---------------------------------------------------------------- documents

/// A doc for a file that doesn't exist yet (saving creates it).
pub fn missing_doc(path: &Path) -> J {
    json!({
        "path": path.display().to_string(), "name": name_of(path), "text": "", "encoding": "utf-8",
        "eol": "lf", "eolDetected": false, "mixedEol": false, "mtime": 0.0, "size": 0,
        "readonly": false, "binary": false, "lossy": false, "missing": true,
    })
}

/// Reads a file into a doc. Err is a full `{ok:false, ...}` reply.
pub fn open_doc(path: &Path, forced: Option<&str>) -> Result<J, J> {
    let name = name_of(path);
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(json!({ "ok": false, "error": format!("Couldn't open {name}: it doesn't exist."), "notFound": true }))
        }
        Err(e) => return Err(err(format!("Couldn't open {name}: {}.", io_msg(&e)))),
    };
    if meta.is_dir() {
        return Err(json!({ "ok": false, "error": format!("{name} is a folder. Use Open Folder to see its files."), "isDir": true }));
    }
    if meta.len() > MAX_OPEN {
        return Err(err(format!("{name} is larger than 100 MB, which Axentra Notepad can't open yet.")));
    }
    let bytes = std::fs::read(path).map_err(|e| err(format!("Couldn't open {name}: {}.", io_msg(&e))))?;
    let dec = decode(&bytes, forced);
    let eol = detect_eol(&dec.text);
    let binary = !dec.encoding.starts_with("utf-16") && dec.text.contains('\0');
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    Ok(json!({
        "path": canonical.display().to_string(),
        "name": name,
        "text": normalize_eol(&dec.text),
        "encoding": dec.encoding,
        "eol": eol.eol,
        "eolDetected": eol.detected,
        "mixedEol": eol.mixed,
        "mtime": mtime_ms(&meta),
        "size": meta.len(),
        "readonly": readonly(path),
        "binary": binary,
        "lossy": dec.lossy,
    }))
}

pub fn save_doc(path: &Path, text: &str, encoding: &str, eol: &str) -> Result<J, J> {
    let name = name_of(path);
    if path.is_dir() {
        return Err(err(format!("Couldn't save {name}: a folder has that name.")));
    }
    // Write through symlinks to the file they point at instead of replacing the link.
    let target = if path.exists() { std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()) } else { path.to_path_buf() };
    if target.exists() && readonly(&target) {
        return Err(json!({ "ok": false, "error": format!("Couldn't save {name}: the file is read-only. Use Save As to keep a copy somewhere else."), "readonly": true }));
    }
    let id = if ENCODINGS.iter().any(|e| e.0 == encoding) { encoding } else { "utf-8" };
    let bytes = encode(&apply_eol(text, eol), id).map_err(|c| {
        json!({
            "ok": false,
            "error": format!("{name} can't be saved as {} because it contains characters like “{c}”. Choose UTF-8 under Encoding to keep every character.", encoding_name(id)),
            "unencodable": true,
        })
    })?;
    atomic_write(&target, &bytes).map_err(|e| err(format!("Couldn't save {name}: {e}.")))?;
    let canonical = std::fs::canonicalize(&target).unwrap_or(target);
    let meta = std::fs::metadata(&canonical).map_err(|e| err(format!("Couldn't save {name}: {}.", io_msg(&e))))?;
    Ok(json!({ "path": canonical.display().to_string(), "name": name, "mtime": mtime_ms(&meta), "size": meta.len() }))
}

pub fn stat(paths: &[String]) -> J {
    let mut out = Map::new();
    for p in paths {
        let path = Path::new(p);
        let v = match std::fs::metadata(path) {
            Ok(m) => json!({ "exists": true, "mtime": mtime_ms(&m), "size": m.len(), "readonly": readonly(path), "dir": m.is_dir() }),
            Err(_) => json!({ "exists": false, "mtime": 0.0, "size": 0, "readonly": false, "dir": false }),
        };
        out.insert(p.clone(), v);
    }
    J::Object(out)
}

// ---------------------------------------------------------------- recent lists

fn read_list(file: &Path) -> Vec<String> {
    std::fs::read(file).ok().and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok()).unwrap_or_default()
}

fn write_list(file: &Path, list: &[String]) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(list) {
        let _ = atomic_write(file, &json);
    }
}

pub fn recent(data: &Path, folders: bool) -> Vec<String> {
    let (file, want_dir) = if folders { (data.join("recent-folders.json"), true) } else { (data.join("recent.json"), false) };
    read_list(&file).into_iter().filter(|p| if want_dir { Path::new(p).is_dir() } else { Path::new(p).is_file() }).collect()
}

pub fn remember(data: &Path, path: &Path, folder: bool) {
    let Ok(path) = std::fs::canonicalize(path) else { return };
    let p = path.display().to_string();
    let (file, max) = if folder { (data.join("recent-folders.json"), 10) } else { (data.join("recent.json"), 20) };
    let mut list = read_list(&file);
    list.retain(|x| *x != p);
    list.insert(0, p);
    list.truncate(max);
    write_list(&file, &list);
}

pub fn forget(data: &Path, path: &str) {
    for name in ["recent.json", "recent-folders.json"] {
        let file = data.join(name);
        let mut list = read_list(&file);
        let before = list.len();
        list.retain(|x| x != path);
        if list.len() != before {
            write_list(&file, &list);
        }
    }
}

pub fn clear_recent(data: &Path, kind: &str) {
    if kind != "folders" {
        write_list(&data.join("recent.json"), &[]);
    }
    if kind != "files" {
        write_list(&data.join("recent-folders.json"), &[]);
    }
}

// ---------------------------------------------------------------- session

pub fn valid_buffer_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn save_session(data: &Path, session: &J, buffers: &Map<String, J>, drop: &[String]) -> Result<(), String> {
    if let Some(bad) = buffers.keys().chain(drop.iter()).find(|id| !valid_buffer_id(id)) {
        return Err(format!("“{bad}” isn't a valid backup name."));
    }
    let backup = data.join("backup");
    std::fs::create_dir_all(&backup).map_err(|e| format!("Couldn't save the session: {}.", io_msg(&e)))?;
    for (id, text) in buffers {
        let text = text.as_str().unwrap_or("");
        atomic_write(&backup.join(format!("{id}.txt")), text.as_bytes()).map_err(|e| format!("Couldn't back up an unsaved tab: {e}."))?;
    }
    for id in drop {
        let _ = std::fs::remove_file(backup.join(format!("{id}.txt")));
    }
    let json = serde_json::to_vec(session).map_err(|e| e.to_string())?;
    atomic_write(&data.join("session.json"), &json).map_err(|e| format!("Couldn't save the session: {e}."))
}

/// The saved session and every backup buffer that exists.
pub fn load_session(data: &Path) -> (J, Map<String, J>) {
    let session = std::fs::read(data.join("session.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(J::Null);
    let mut buffers = Map::new();
    if let Ok(dir) = std::fs::read_dir(data.join("backup")) {
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".txt") else { continue };
            if !valid_buffer_id(id) {
                continue;
            }
            if let Ok(bytes) = std::fs::read(entry.path()) {
                buffers.insert(id.to_string(), J::String(String::from_utf8_lossy(&bytes).into_owned()));
            }
        }
    }
    (session, buffers)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn temp(tag: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("axn-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_and_round_trips_every_encoding() {
        let samples: &[(&str, &str)] = &[
            ("utf-8", "héllo wörld € 你好 😀"),
            ("utf-8-bom", "héllo € 😀"),
            ("utf-16le", "héllo € 你好 😀"),
            ("utf-16be", "héllo € 你好 😀"),
            ("windows-1252", "café € “quotes”"),
            ("iso-8859-15", "café €"),
            ("windows-1250", "Příliš žluťoučký"),
            ("iso-8859-2", "Příliš žluťoučký"),
            ("windows-1251", "Привет мир"),
            ("koi8-r", "Привет мир"),
            ("ibm866", "Привет мир"),
            ("windows-1253", "Καλημέρα"),
            ("windows-1254", "Günaydın şğ"),
            ("windows-1255", "שלום"),
            ("windows-1256", "مرحبا"),
            ("windows-1257", "Labrīt ąčę"),
            ("windows-1258", "Xin chào"),
            ("windows-874", "สวัสดี"),
            ("shift_jis", "こんにちは世界"),
            ("euc-jp", "こんにちは世界"),
            ("gbk", "你好世界"),
            ("gb18030", "你好世界 😀"),
            ("big5", "你好世界"),
            ("euc-kr", "안녕하세요"),
            ("macintosh", "café ©"),
        ];
        assert_eq!(samples.len(), ENCODINGS.len());
        for (id, text) in samples {
            let bytes = encode(text, id).unwrap_or_else(|c| panic!("{id} can't encode {c:?}"));
            let back = decode(&bytes, Some(id));
            assert_eq!(back.text, *text, "{id}");
            assert_eq!(back.encoding, *id);
            assert!(!back.lossy, "{id}");
        }
        // Detection without being told.
        assert_eq!(decode("plain ascii".as_bytes(), None).encoding, "utf-8");
        assert_eq!(decode("ünïcode".as_bytes(), None).encoding, "utf-8");
        assert_eq!(decode(&encode("x", "utf-8-bom").unwrap(), None).encoding, "utf-8-bom");
        assert_eq!(decode(&encode("x", "utf-8-bom").unwrap(), None).text, "x");
        assert_eq!(decode(&encode("hi €", "utf-16le").unwrap(), None).encoding, "utf-16le");
        assert_eq!(decode(&encode("hi €", "utf-16be").unwrap(), None).encoding, "utf-16be");
        let bomless: Vec<u8> = "Hello, world\r\n".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let d = decode(&bomless, None);
        assert_eq!((d.encoding, d.text.as_str()), ("utf-16le", "Hello, world\r\n"));
        let bomless_be: Vec<u8> = "Hello, world".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert_eq!(decode(&bomless_be, None).encoding, "utf-16be");
        let ansi = b"caf\xe9 \x80 \x93q\x94";
        let d = decode(ansi, None);
        assert_eq!((d.encoding, d.text.as_str(), d.lossy), ("windows-1252", "café € “q”", false));
        // Forcing UTF-8 on bytes that aren't marks the decode lossy.
        assert!(decode(ansi, Some("utf-8")).lossy);
        // Unknown ids fall back to detection.
        assert_eq!(decode(b"abc", Some("klingon")).encoding, "utf-8");
    }

    #[test]
    fn unencodable_characters_are_named() {
        assert_eq!(encode("price: 5 € or ₹", "iso-8859-2"), Err('€'));
        assert_eq!(encode("abc 😀", "windows-1252"), Err('😀'));
        let dir = temp("unenc");
        let path = dir.join("a.txt");
        let e = save_doc(&path, "naïve ☃", "windows-1252", "lf").unwrap_err();
        assert_eq!(e["unencodable"], true);
        let msg = e["error"].as_str().unwrap();
        assert!(msg.contains("Western (Windows 1252)") && msg.contains("“☃”"), "{msg}");
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn line_endings() {
        let e = detect_eol("a\r\nb\r\nc");
        assert_eq!((e.eol, e.detected, e.mixed), ("crlf", true, false));
        let e = detect_eol("a\nb\n");
        assert_eq!((e.eol, e.detected, e.mixed), ("lf", true, false));
        let e = detect_eol("a\rb\r");
        assert_eq!((e.eol, e.detected, e.mixed), ("cr", true, false));
        let e = detect_eol("a\r\nb\nc\r\n");
        assert_eq!((e.eol, e.detected, e.mixed), ("crlf", true, true));
        let e = detect_eol("one line");
        assert_eq!((e.eol, e.detected, e.mixed), ("lf", false, false));
        assert_eq!(normalize_eol("a\r\nb\rc\n"), "a\nb\nc\n");
        assert_eq!(apply_eol("a\nb\n", "crlf"), "a\r\nb\r\n");
        assert_eq!(apply_eol("a\nb", "cr"), "a\rb");
        assert_eq!(apply_eol("a\nb", "lf"), "a\nb");
    }

    #[test]
    fn open_save_round_trip_keeps_permissions_and_eol() {
        let dir = temp("save");
        let path = dir.join("notes.txt");
        std::fs::write(&path, b"one\r\ntwo\r\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        let doc = open_doc(&path, None).unwrap();
        assert_eq!(doc["text"], "one\ntwo\n");
        assert_eq!(doc["eol"], "crlf");
        assert_eq!(doc["encoding"], "utf-8");
        assert_eq!(doc["readonly"], false);
        assert_eq!(doc["binary"], false);
        let saved = save_doc(&path, "uno\ndos\n", "utf-16le", "crlf").unwrap();
        assert_eq!(saved["name"], "notes.txt");
        assert!(saved["mtime"].as_f64().unwrap() > 0.0);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes, encode("uno\r\ndos\r\n", "utf-16le").unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
        }
        let again = open_doc(&path, None).unwrap();
        assert_eq!((again["text"].as_str(), again["encoding"].as_str()), (Some("uno\ndos\n"), Some("utf-16le")));
        // No temp files left behind.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        // Missing, folders and binary files.
        let missing = open_doc(&dir.join("nope.txt"), None).unwrap_err();
        assert_eq!(missing["notFound"], true);
        assert_eq!(missing["error"], "Couldn't open nope.txt: it doesn't exist.");
        assert_eq!(open_doc(&dir, None).unwrap_err()["isDir"], true);
        std::fs::write(dir.join("bin.dat"), b"\x7fELF\0\0\x01\x02abc").unwrap();
        assert_eq!(open_doc(&dir.join("bin.dat"), None).unwrap()["binary"], true);
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_a_symlink_keeps_the_link() {
        let dir = temp("link");
        let real = dir.join("real.txt");
        std::fs::write(&real, "old").unwrap();
        std::os::unix::fs::symlink(&real, dir.join("link.txt")).unwrap();
        save_doc(&dir.join("link.txt"), "new", "utf-8", "lf").unwrap();
        assert!(std::fs::symlink_metadata(dir.join("link.txt")).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn stat_reports_files() {
        let dir = temp("stat");
        let a = dir.join("a.txt");
        std::fs::write(&a, "12345").unwrap();
        let s = stat(&[a.display().to_string(), dir.join("b").display().to_string()]);
        assert_eq!(s[a.display().to_string()]["exists"], true);
        assert_eq!(s[a.display().to_string()]["size"], 5);
        assert_eq!(s[dir.join("b").display().to_string()]["exists"], false);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn recent_lists() {
        let data = temp("recent");
        let f = data.join("x.txt");
        std::fs::write(&f, "").unwrap();
        remember(&data, &f, false);
        remember(&data, &data, true);
        assert_eq!(recent(&data, false), vec![std::fs::canonicalize(&f).unwrap().display().to_string()]);
        assert_eq!(recent(&data, true).len(), 1);
        forget(&data, &std::fs::canonicalize(&f).unwrap().display().to_string());
        assert!(recent(&data, false).is_empty());
        clear_recent(&data, "all");
        assert!(recent(&data, true).is_empty());
        std::fs::remove_dir_all(data).ok();
    }

    #[test]
    fn session_backup() {
        let data = temp("session");
        let mut buffers = Map::new();
        buffers.insert("tab-1".into(), json!("unsaved ✓"));
        buffers.insert("tab_2".into(), json!("two"));
        let session = json!({ "tabs": [{ "id": "tab-1" }], "active": 0 });
        save_session(&data, &session, &buffers, &[]).unwrap();
        let (s, b) = load_session(&data);
        assert_eq!(s, session);
        assert_eq!(b["tab-1"], "unsaved ✓");
        assert_eq!(b.len(), 2);
        save_session(&data, &session, &Map::new(), &["tab_2".into()]).unwrap();
        assert_eq!(load_session(&data).1.len(), 1);
        let mut bad = Map::new();
        bad.insert("../escape".into(), json!("x"));
        assert!(save_session(&data, &session, &bad, &[]).is_err());
        assert!(save_session(&data, &session, &Map::new(), &["a/b".into()]).is_err());
        assert!(!valid_buffer_id("") && !valid_buffer_id(&"a".repeat(65)) && valid_buffer_id("Az09_-"));
        std::fs::remove_dir_all(data).ok();
    }
}
