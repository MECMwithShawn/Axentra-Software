//! Folders: quick-open listings, find in files and replace in files (ripgrep's walker).
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::{WalkBuilder, WalkState};
use regex::{NoExpand, Regex, RegexBuilder};
use serde_json::{json, Value as J};

use crate::files::{self, atomic_write, io_msg, name_of};

/// Never listed or searched, whatever the ignore files say.
const SKIP: &[&str] = &[".git", ".hg", ".svn", "node_modules", "__pycache__", ".DS_Store"];
const MAX_SEARCH_FILE: u64 = 16 * 1024 * 1024;
/// Longer lines are cut to a window around the first match.
const LONG_LINE: usize = 400;

fn walker(root: &Path, use_ignore: bool) -> WalkBuilder {
    let mut w = WalkBuilder::new(root);
    w.hidden(false)
        .follow_links(false)
        .ignore(use_ignore)
        .git_ignore(use_ignore)
        .git_global(use_ignore)
        .git_exclude(use_ignore)
        .parents(use_ignore)
        .require_git(false)
        .filter_entry(|e| !SKIP.iter().any(|s| e.file_name() == *s));
    w
}

fn rel_of(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

pub fn list_files(root: &Path, limit: usize) -> Result<J, String> {
    if !root.is_dir() {
        return Err(format!("Couldn't list {}: it isn't a folder.", name_of(root)));
    }
    let mut files = Vec::new();
    let mut truncated = false;
    for entry in walker(root, true).build().flatten() {
        if entry.file_type().is_some_and(|t| t.is_file() || t.is_symlink()) && entry.depth() > 0 {
            if files.len() >= limit {
                truncated = true;
                break;
            }
            files.push(rel_of(root, entry.path()));
        }
    }
    files.sort();
    Ok(json!({ "files": files, "truncated": truncated }))
}

// ---------------------------------------------------------------- globs

/// VS Code style include/exclude lists: `*.rs` or `target` match a name anywhere,
/// `src/**/*.ts` matches the path from the folder.
pub struct Globs {
    names: GlobSet,
    paths: GlobSet,
    empty: bool,
}

impl Globs {
    pub fn parse(list: &str) -> Result<Globs, String> {
        let (mut names, mut paths) = (GlobSetBuilder::new(), GlobSetBuilder::new());
        let mut empty = true;
        for raw in list.split(',') {
            let p = raw.trim();
            if p.is_empty() {
                continue;
            }
            empty = false;
            let bad = |e: globset::Error| format!("“{p}” isn't a valid pattern: {}", e.kind());
            let p = p.trim_start_matches("./").trim_start_matches('/').trim_end_matches('/');
            if p.contains('/') {
                let g = |s: &str| GlobBuilder::new(s).literal_separator(true).build();
                paths.add(g(p).map_err(bad)?);
                // A folder path also covers everything inside it.
                if !p.ends_with("**") {
                    paths.add(g(&format!("{p}/**")).map_err(bad)?);
                }
            } else {
                names.add(Glob::new(p).map_err(bad)?);
            }
        }
        Ok(Globs { names: names.build().map_err(|e| e.to_string())?, paths: paths.build().map_err(|e| e.to_string())?, empty })
    }

    pub fn is_empty(&self) -> bool {
        self.empty
    }

    /// Matches a "/"-separated path relative to the root.
    pub fn matches(&self, rel: &str) -> bool {
        !self.empty && (self.paths.is_match(rel) || rel.split('/').any(|seg| self.names.is_match(seg)))
    }
}

// ---------------------------------------------------------------- search

pub fn build_regex(query: &str, regex: bool, case: bool, word: bool) -> Result<Regex, String> {
    if query.is_empty() {
        return Err("Type something to search for.".into());
    }
    let mut pat = if regex { query.to_string() } else { regex::escape(query) };
    if word {
        pat = format!(r"\b(?:{pat})\b");
    }
    RegexBuilder::new(&pat).case_insensitive(!case).size_limit(64 << 20).build().map_err(|e| {
        let text = e.to_string();
        let detail = text.lines().rev().find(|l| l.trim_start().starts_with("error:")).map(|l| l.trim().trim_start_matches("error:").trim().to_string());
        format!("That isn't a valid regular expression: {}.", detail.unwrap_or_else(|| text.lines().next().unwrap_or("").to_string()).trim_end_matches('.'))
    })
}

/// Text for searching, or None for binary files.
fn searchable(bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(b"\xFF\xFE") || bytes.starts_with(b"\xFE\xFF") {
        return Some(files::decode(bytes, None).text);
    }
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return None;
    }
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    Some(String::from_utf8_lossy(b).into_owned())
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// One line's matches: the shown text and UTF-16 ranges inside it.
fn line_matches(line: &str, re: &Regex, line_no: usize, budget: usize) -> Option<(J, usize)> {
    let found: Vec<(usize, usize)> = re.find_iter(line).take(budget).map(|m| (m.start(), m.end())).collect();
    if found.is_empty() {
        return None;
    }
    // Byte offsets → UTF-16 columns, walking the line once.
    let mut cols = Vec::with_capacity(found.len());
    let (mut byte, mut col) = (0usize, 0usize);
    let mut to_col = |target: usize| {
        col += utf16_len(&line[byte..target]);
        byte = target;
        col
    };
    for &(s, e) in &found {
        let a = to_col(s);
        let b = to_col(e.max(s));
        cols.push((a, b));
    }
    let total = utf16_len(line);
    let first = cols[0].0;
    let (text, offset) = if total <= LONG_LINE {
        (line.to_string(), 0isize)
    } else {
        // A window of whole chars around the first match.
        let chars: Vec<char> = line.chars().collect();
        let mut start_col = first.saturating_sub(60);
        let mut c = 0usize;
        let mut start_char = 0usize;
        for (i, ch) in chars.iter().enumerate() {
            if c >= start_col {
                start_char = i;
                start_col = c;
                break;
            }
            c += ch.len_utf16();
        }
        let mut text: String = if start_char > 0 { "…".into() } else { String::new() };
        let mut width = 0usize;
        let mut end_char = start_char;
        while end_char < chars.len() && width < LONG_LINE - 2 {
            width += chars[end_char].len_utf16();
            text.push(chars[end_char]);
            end_char += 1;
        }
        let end_col = start_col + width;
        if end_char < chars.len() {
            text.push('…');
        }
        let shift = start_col as isize - if start_char > 0 { 1 } else { 0 };
        let len = text.chars().map(char::len_utf16).sum::<usize>() as isize;
        let ranges: Vec<J> = cols
            .iter()
            .filter(|(a, b)| *b >= start_col && *a <= end_col)
            .map(|&(a, b)| {
                let a = (a as isize - shift).clamp(0, len);
                let b = (b.min(end_col) as isize - shift).clamp(a, len);
                json!([a, b])
            })
            .collect();
        return Some((json!({ "line": line_no, "col": first, "text": text, "ranges": ranges }), found.len()));
    };
    let ranges: Vec<J> = cols.iter().map(|&(a, b)| json!([a as isize - offset, b as isize - offset])).collect();
    Some((json!({ "line": line_no, "col": first, "text": text, "ranges": ranges }), found.len()))
}

/// A file's matching lines, how many matches, and whether the budget cut it short.
fn search_text(text: &str, re: &Regex, budget: usize) -> (Vec<J>, usize, bool) {
    let mut out = Vec::new();
    let mut count = 0;
    for (i, raw) in text.split('\n').enumerate() {
        if count >= budget {
            return (out, count, true);
        }
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some((m, n)) = line_matches(line, re, i, budget - count) {
            out.push(m);
            count += n;
        }
    }
    (out, count, false)
}

/// One file's results: relative path, path, matching lines and the match count.
type FileHits = (String, PathBuf, Vec<J>, usize);

pub struct SearchOpts<'a> {
    pub query: &'a str,
    pub regex: bool,
    pub case: bool,
    pub word: bool,
    pub include: &'a str,
    pub exclude: &'a str,
    pub use_ignore: bool,
    pub max_results: usize,
}

impl SearchOpts<'_> {
    pub fn from_json(req: &J) -> SearchOpts<'_> {
        SearchOpts {
            query: req["query"].as_str().unwrap_or(""),
            regex: req["regex"].as_bool().unwrap_or(false),
            case: req["case"].as_bool().unwrap_or(false),
            word: req["word"].as_bool().unwrap_or(false),
            include: req["include"].as_str().unwrap_or(""),
            exclude: req["exclude"].as_str().unwrap_or(""),
            use_ignore: req["useIgnore"].as_bool().unwrap_or(true),
            max_results: req["maxResults"].as_u64().map(|n| n as usize).unwrap_or(10_000).clamp(1, 200_000),
        }
    }
}

pub fn search(root: &Path, o: &SearchOpts) -> Result<J, String> {
    if !root.is_dir() {
        return Err(format!("Couldn't search {}: it isn't a folder.", name_of(root)));
    }
    let re = build_regex(o.query, o.regex, o.case, o.word)?;
    let include = Globs::parse(o.include)?;
    let exclude = Globs::parse(o.exclude)?;
    let found: Mutex<Vec<FileHits>> = Mutex::new(Vec::new());
    let total = AtomicUsize::new(0);
    let skipped = AtomicUsize::new(0);
    let files_seen = AtomicUsize::new(0);
    let cut = AtomicBool::new(false);
    let max = o.max_results;
    let root_owned = root.to_path_buf();
    let mut w = walker(root, o.use_ignore);
    let prune_root = root_owned.clone();
    // Excluded folders are pruned instead of walked.
    w.filter_entry(move |e| !SKIP.iter().any(|s| e.file_name() == *s) && (e.depth() == 0 || !exclude.matches(&rel_of(&prune_root, e.path()))));
    w.build_parallel().run(|| {
        let (re, include, found, total, skipped, files_seen, cut, root) = (&re, &include, &found, &total, &skipped, &files_seen, &cut, &root_owned);
        Box::new(move |entry| {
            if total.load(Ordering::Relaxed) >= max {
                cut.store(true, Ordering::Relaxed);
                return WalkState::Quit;
            }
            let Ok(entry) = entry else { return WalkState::Continue };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                return WalkState::Continue;
            }
            let rel = rel_of(root, entry.path());
            if !include.is_empty() && !include.matches(&rel) {
                return WalkState::Continue;
            }
            files_seen.fetch_add(1, Ordering::Relaxed);
            let too_big = entry.metadata().map(|m| m.len() > MAX_SEARCH_FILE).unwrap_or(false);
            let text = if too_big { None } else { std::fs::read(entry.path()).ok().and_then(|b| searchable(&b)) };
            let Some(text) = text else {
                skipped.fetch_add(1, Ordering::Relaxed);
                return WalkState::Continue;
            };
            let (matches, n, file_cut) = search_text(&text, re, max);
            if file_cut {
                cut.store(true, Ordering::Relaxed);
            }
            if n > 0 {
                total.fetch_add(n, Ordering::Relaxed);
                found.lock().unwrap().push((rel, entry.path().to_path_buf(), matches, n));
            }
            WalkState::Continue
        })
    });
    let mut found = found.into_inner().unwrap();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    // Parallel workers may overshoot; keep whole files in path order up to the cap.
    let mut results = Vec::new();
    let mut count = 0usize;
    let mut truncated = cut.load(Ordering::Relaxed);
    for (rel, path, mut matches, n) in found {
        if count >= max {
            truncated = true;
            break;
        }
        if count + n > max {
            let mut keep = Vec::new();
            for mut m in matches.drain(..) {
                let k = m["ranges"].as_array().map(|r| r.len()).unwrap_or(1).max(1);
                if count + k > max {
                    let room = max - count;
                    if room > 0 {
                        if let Some(r) = m["ranges"].as_array_mut() {
                            r.truncate(room);
                        }
                        count += room;
                        keep.push(m);
                    }
                    break;
                }
                count += k;
                keep.push(m);
            }
            matches = keep;
            truncated = true;
            if matches.is_empty() {
                break;
            }
        } else {
            count += n;
        }
        results.push(json!({ "path": path.display().to_string(), "rel": rel, "matches": matches }));
    }
    Ok(json!({
        "results": results,
        "fileCount": results.len(),
        "matchCount": count,
        "truncated": truncated,
        "skipped": skipped.load(Ordering::Relaxed),
        "searched": files_seen.load(Ordering::Relaxed),
    }))
}

// ---------------------------------------------------------------- replace

pub fn replace_in_files(root: &Path, o: &SearchOpts, replacement: &str, paths: &[String]) -> Result<J, String> {
    let re = build_regex(o.query, o.regex, o.case, o.word)?;
    let root = std::fs::canonicalize(root).map_err(|e| format!("Couldn't open the folder: {}.", io_msg(&e)))?;
    let mut changed = Vec::new();
    let mut skipped = Vec::new();
    for p in paths {
        let mut skip = |reason: String| skipped.push(json!({ "path": p, "reason": reason }));
        let path = match std::fs::canonicalize(p) {
            Ok(path) => path,
            Err(e) => {
                skip(format!("Couldn't open it: {}.", io_msg(&e)));
                continue;
            }
        };
        if !path.starts_with(&root) {
            skip("It isn't in the folder being searched.".into());
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                skip(format!("Couldn't read it: {}.", io_msg(&e)));
                continue;
            }
        };
        let (text, enc) = if bytes.starts_with(b"\xEF\xBB\xBF") || bytes.starts_with(b"\xFF\xFE") || bytes.starts_with(b"\xFE\xFF") {
            let d = files::decode(&bytes, None);
            (d.text, d.encoding)
        } else if bytes[..bytes.len().min(8192)].contains(&0) {
            skip("It looks like a binary file.".into());
            continue;
        } else {
            match String::from_utf8(bytes) {
                Ok(s) => (s, "utf-8"),
                Err(_) => {
                    skip("It isn't UTF-8 or UTF-16, so it was left alone. Open it to replace in it.".into());
                    continue;
                }
            }
        };
        // Line by line, like the search, so line endings stay exactly as they were.
        let mut out = String::with_capacity(text.len());
        let mut count = 0usize;
        for piece in text.split_inclusive('\n') {
            let (body, end) = match piece.strip_suffix("\r\n") {
                Some(b) => (b, "\r\n"),
                None => match piece.strip_suffix('\n') {
                    Some(b) => (b, "\n"),
                    None => (piece, ""),
                },
            };
            let n = re.find_iter(body).count();
            if n == 0 {
                out.push_str(piece);
                continue;
            }
            count += n;
            let replaced = if o.regex { re.replace_all(body, replacement) } else { re.replace_all(body, NoExpand(replacement)) };
            out.push_str(&replaced);
            out.push_str(end);
        }
        if count == 0 {
            skip("It no longer has any matches.".into());
            continue;
        }
        let bytes = files::encode(&out, enc).unwrap_or_else(|_| out.clone().into_bytes());
        if let Err(e) = atomic_write(&path, &bytes) {
            skip(format!("Couldn't save it: {e}."));
            continue;
        }
        let mtime = std::fs::metadata(&path).map(|m| files::mtime_ms(&m)).unwrap_or(0.0);
        changed.push(json!({ "path": path.display().to_string(), "count": count, "mtime": mtime }));
    }
    Ok(json!({ "changed": changed, "skipped": skipped }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::tests::temp;

    fn opts(query: &str) -> SearchOpts<'_> {
        SearchOpts { query, regex: false, case: false, word: false, include: "", exclude: "", use_ignore: true, max_results: 10_000 }
    }

    fn tree(tag: &str) -> PathBuf {
        let root = temp(tag);
        let w = |rel: &str, body: &[u8]| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        w("src/main.rs", b"fn main() {\n    println!(\"a.b(c)\");\n    let Foo = foo + FOO;\n}\n");
        w("src/lib/util.ts", b"export const foo = 1; // foobar\r\nfoo();\r\n");
        w("README.md", "# Foo\n😀 中文 foo here\n".as_bytes());
        w("target/debug/out.txt", b"foo in a build folder\n");
        w("node_modules/x/index.js", b"foo\n");
        w(".git/config", b"foo\n");
        w(".gitignore", b"target/\n*.log\n");
        w("debug.log", b"foo\n");
        w("image.bin", b"foo\0\0\0binary");
        w(".hidden/notes.txt", b"foo hidden\n");
        root
    }

    fn rels(r: &J) -> Vec<String> {
        r["results"].as_array().unwrap().iter().map(|x| x["rel"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn list_files_respects_ignores() {
        let root = tree("list");
        let r = list_files(&root, 100).unwrap();
        let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
        assert_eq!(files, vec![".gitignore", ".hidden/notes.txt", "README.md", "image.bin", "src/lib/util.ts", "src/main.rs"]);
        assert_eq!(r["truncated"], false);
        assert_eq!(list_files(&root, 2).unwrap()["truncated"], true);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn search_options() {
        let root = tree("search");
        let r = search(&root, &opts("foo")).unwrap();
        assert_eq!(rels(&r), vec![".hidden/notes.txt", "README.md", "src/lib/util.ts", "src/main.rs"]);
        assert_eq!(r["skipped"], 1, "image.bin is binary");
        assert_eq!(r["truncated"], false);
        // Case and whole word.
        let r = search(&root, &SearchOpts { case: true, ..opts("foo") }).unwrap();
        let main = &r["results"][3]["matches"];
        assert_eq!(main[0]["ranges"], json!([[14, 17]]));
        let r = search(&root, &SearchOpts { word: true, ..opts("foo") }).unwrap();
        let util = &r["results"][2]["matches"][0];
        assert_eq!(util["ranges"].as_array().unwrap().len(), 1, "foobar isn't a whole word");
        // Literal metacharacters and regex.
        let r = search(&root, &opts("a.b(c)")).unwrap();
        assert_eq!(rels(&r), vec!["src/main.rs"]);
        assert_eq!(r["results"][0]["matches"][0]["line"], 1);
        let r = search(&root, &SearchOpts { regex: true, ..opts(r"fo+\(\)") }).unwrap();
        assert_eq!(rels(&r), vec!["src/lib/util.ts"]);
        assert_eq!(r["results"][0]["matches"][0]["text"], "foo();", "CR stripped");
        assert!(search(&root, &SearchOpts { regex: true, ..opts("(unclosed") }).unwrap_err().starts_with("That isn't a valid regular expression"));
        // Include and exclude.
        let r = search(&root, &SearchOpts { include: "*.rs, *.md", ..opts("foo") }).unwrap();
        assert_eq!(rels(&r), vec!["README.md", "src/main.rs"]);
        let r = search(&root, &SearchOpts { include: "src/**/*.ts", ..opts("foo") }).unwrap();
        assert_eq!(rels(&r), vec!["src/lib/util.ts"]);
        let r = search(&root, &SearchOpts { include: "src", ..opts("foo") }).unwrap();
        assert_eq!(rels(&r), vec!["src/lib/util.ts", "src/main.rs"]);
        let r = search(&root, &SearchOpts { exclude: "lib, .hidden", ..opts("foo") }).unwrap();
        assert_eq!(rels(&r), vec!["README.md", "src/main.rs"]);
        let r = search(&root, &SearchOpts { exclude: "src/lib", ..opts("foo") }).unwrap();
        assert!(!rels(&r).contains(&"src/lib/util.ts".to_string()));
        // Ignore files can be turned off (but .git and node_modules stay out).
        let r = search(&root, &SearchOpts { use_ignore: false, ..opts("foo") }).unwrap();
        assert!(rels(&r).contains(&"target/debug/out.txt".to_string()) && rels(&r).contains(&"debug.log".to_string()));
        assert!(!rels(&r).iter().any(|p| p.starts_with(".git/") || p.starts_with("node_modules/")));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn utf16_columns_and_long_lines() {
        let root = tree("cols");
        let r = search(&root, &SearchOpts { include: "README.md", ..opts("foo") }).unwrap();
        let m = &r["results"][0]["matches"];
        // "😀 中文 foo": the emoji is two UTF-16 units, so foo starts at column 6.
        assert_eq!(m[1]["col"], 6);
        assert_eq!(m[1]["ranges"], json!([[6, 9]]));
        let long = format!("{}needle{}", "x".repeat(1000), "y".repeat(1000));
        std::fs::write(root.join("long.txt"), &long).unwrap();
        let r = search(&root, &SearchOpts { include: "long.txt", ..opts("needle") }).unwrap();
        let m = &r["results"][0]["matches"][0];
        assert_eq!(m["col"], 1000);
        let text = m["text"].as_str().unwrap();
        assert!(text.starts_with('…') && text.ends_with('…'));
        assert!(text.encode_utf16().count() <= LONG_LINE);
        let range = m["ranges"][0].as_array().unwrap();
        let (a, b) = (range[0].as_u64().unwrap() as usize, range[1].as_u64().unwrap() as usize);
        let units: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(String::from_utf16(&units[a..b]).unwrap(), "needle");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn truncates_at_max_results() {
        let root = temp("trunc");
        for i in 0..20 {
            std::fs::write(root.join(format!("f{i:02}.txt")), "hit hit hit\nhit\n").unwrap();
        }
        let r = search(&root, &SearchOpts { max_results: 10, ..opts("hit") }).unwrap();
        assert_eq!(r["truncated"], true);
        assert_eq!(r["matchCount"], 10);
        let r = search(&root, &opts("hit")).unwrap();
        assert_eq!((r["matchCount"].as_u64(), r["fileCount"].as_u64(), r["truncated"].as_bool()), (Some(80), Some(20), Some(false)));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn replaces_in_files_keeping_encoding_and_eol() {
        let root = temp("replace");
        let crlf = root.join("crlf.txt");
        std::fs::write(&crlf, b"cost $1 = foo\r\nfoo foo\r\n").unwrap();
        let u16 = root.join("wide.txt");
        std::fs::write(&u16, files::encode("foo\nbar foo\n", "utf-16le").unwrap()).unwrap();
        let ansi = root.join("ansi.txt");
        std::fs::write(&ansi, b"foo caf\xe9\n").unwrap();
        let bin = root.join("bin.dat");
        std::fs::write(&bin, b"foo\0\0").unwrap();
        let none = root.join("none.txt");
        std::fs::write(&none, b"nothing here\n").unwrap();
        let outside = temp("outside").join("o.txt");
        std::fs::write(&outside, b"foo\n").unwrap();
        let all: Vec<String> = [&crlf, &u16, &ansi, &bin, &none, &outside].iter().map(|p| p.display().to_string()).collect();
        let r = replace_in_files(&root, &opts("foo"), "$1x", &all).unwrap();
        assert_eq!(r["changed"].as_array().unwrap().len(), 2);
        assert_eq!(r["changed"][0]["count"], 3);
        assert_eq!(std::fs::read(&crlf).unwrap(), b"cost $1 = $1x\r\n$1x $1x\r\n");
        assert_eq!(files::decode(&std::fs::read(&u16).unwrap(), None).text, "$1x\nbar $1x\n");
        assert!(std::fs::read(&u16).unwrap().starts_with(b"\xFF\xFE"));
        let reasons: Vec<&str> = r["skipped"].as_array().unwrap().iter().map(|s| s["reason"].as_str().unwrap()).collect();
        assert_eq!(reasons.len(), 4);
        assert!(reasons[0].contains("isn't UTF-8") && reasons[1].contains("binary") && reasons[2].contains("no longer") && reasons[3].contains("isn't in the folder"));
        assert_eq!(std::fs::read(&outside).unwrap(), b"foo\n");
        // Regex replacements expand groups.
        std::fs::write(&none, b"key=value\n").unwrap();
        let o = SearchOpts { regex: true, ..opts(r"(\w+)=(?P<v>\w+)") };
        replace_in_files(&root, &o, "${v}: $1", &[none.display().to_string()]).unwrap();
        assert_eq!(std::fs::read_to_string(&none).unwrap(), "value: key\n");
        std::fs::remove_dir_all(root).ok();
        std::fs::remove_dir_all(outside.parent().unwrap()).ok();
    }

    #[test]
    fn globs() {
        let g = Globs::parse(" *.rs , target,src/**/*.ts, docs/ ").unwrap();
        assert!(g.matches("a/b/c.rs") && g.matches("target/x.txt") && g.matches("x/target/y") && g.matches("src/a/b.ts"));
        assert!(g.matches("docs/readme.md"));
        assert!(!g.matches("main.c") && !g.matches("lib/a.ts"));
        assert!(Globs::parse("").unwrap().is_empty());
        assert!(!Globs::parse("").unwrap().matches("x"));
        assert!(Globs::parse("a[").is_err());
    }
}
