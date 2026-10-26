//! What the page asks for. Every request names an `op`; every reply has `ok`.
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value as J};

use crate::{files, fsops, search};

/// What every request can see: where data lives, whether this window owns the session,
/// and what was asked for on the command line.
pub struct Ctx {
    pub data: PathBuf,
    pub primary: bool,
    pub startup: J,
}

/// Turns command-line or dropped paths into docs to open, a folder, and errors to show.
pub fn resolve_paths(data: &Path, paths: &[String]) -> J {
    let mut docs = Vec::new();
    let mut folders = Vec::new();
    let mut errors = Vec::new();
    for p in paths {
        let path = absolute(Path::new(p));
        if path.is_dir() {
            let dir = std::fs::canonicalize(&path).unwrap_or(path);
            files::remember(data, &dir, true);
            folders.push(J::String(dir.display().to_string()));
            continue;
        }
        match files::open_doc(&path, None) {
            Ok(doc) => {
                files::remember(data, &path, false);
                docs.push(doc);
            }
            Err(e) if e["notFound"] == true => {
                // A new file at that path; saving creates it.
                let parent = path.parent().and_then(|d| std::fs::canonicalize(d).ok());
                let path = parent.zip(path.file_name()).map(|(d, n)| d.join(n)).unwrap_or(path);
                docs.push(files::missing_doc(&path));
            }
            Err(e) => errors.push(e["error"].clone()),
        }
    }
    json!({ "docs": docs, "folders": folders, "errors": errors })
}

pub fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.to_path_buf())
    }
}

fn s<'a>(req: &'a J, key: &str) -> &'a str {
    req[key].as_str().unwrap_or("")
}

fn need_path(req: &J, key: &str) -> Result<PathBuf, J> {
    let p = s(req, key);
    if p.is_empty() {
        return Err(files::err("No file was given."));
    }
    Ok(absolute(Path::new(p)))
}

fn wrap(r: Result<J, String>) -> J {
    match r {
        Ok(mut j) => {
            if !j.is_object() {
                j = json!({});
            }
            j["ok"] = json!(true);
            j
        }
        Err(e) => files::err(e),
    }
}

fn strings(v: &J) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

pub fn handle(ctx: &Ctx, req: &J) -> J {
    match dispatch(ctx, req) {
        Ok(j) => j,
        Err(j) => j,
    }
}

fn dispatch(ctx: &Ctx, req: &J) -> Result<J, J> {
    let data = ctx.data.as_path();
    Ok(match s(req, "op") {
        "init" => {
            let (session, buffers) = if ctx.primary { files::load_session(data) } else { (J::Null, Map::new()) };
            json!({
                "ok": true,
                "version": env!("CARGO_PKG_VERSION"),
                "primary": ctx.primary,
                "home": std::env::var("HOME").unwrap_or_default(),
                "sep": "/",
                "recent": files::recent(data, false),
                "recentFolders": files::recent(data, true),
                "session": session,
                "buffers": buffers,
                "startup": ctx.startup,
                "encodings": files::encodings_json(),
                "fonts": fsops::mono_fonts(),
            })
        }
        "open" => {
            let path = need_path(req, "path")?;
            let doc = files::open_doc(&path, req["encoding"].as_str())?;
            files::remember(data, &path, false);
            json!({ "ok": true, "doc": doc })
        }
        "open_paths" => {
            let mut j = resolve_paths(data, &strings(&req["paths"]));
            j["ok"] = json!(true);
            j
        }
        "save" => {
            let path = need_path(req, "path")?;
            let mut j = files::save_doc(&path, s(req, "text"), s(req, "encoding"), s(req, "eol"))?;
            files::remember(data, &path, false);
            j["ok"] = json!(true);
            j
        }
        "stat" => json!({ "ok": true, "files": files::stat(&strings(&req["paths"])) }),
        "session_save" => {
            if !ctx.primary {
                return Ok(json!({ "ok": true, "ignored": true }));
            }
            let empty = Map::new();
            let buffers = req["buffers"].as_object().unwrap_or(&empty);
            wrap(files::save_session(data, &req["session"], buffers, &strings(&req["drop"])).map(|_| json!({})))
        }
        "recent_remove" => {
            files::forget(data, s(req, "path"));
            json!({ "ok": true, "recent": files::recent(data, false), "recentFolders": files::recent(data, true) })
        }
        "recent_clear" => {
            files::clear_recent(data, req["kind"].as_str().unwrap_or("all"));
            json!({ "ok": true, "recent": files::recent(data, false), "recentFolders": files::recent(data, true) })
        }
        "recent_folder_add" => {
            files::remember(data, &need_path(req, "path")?, true);
            json!({ "ok": true, "recentFolders": files::recent(data, true) })
        }
        "list_dir" => wrap(fsops::list_dir(&need_path(req, "path")?)),
        "list_files" => {
            let limit = req["limit"].as_u64().map(|n| n as usize).unwrap_or(20_000).clamp(1, 500_000);
            wrap(search::list_files(&need_path(req, "root")?, limit))
        }
        "search" => wrap(search::search(&need_path(req, "root")?, &search::SearchOpts::from_json(req))),
        "replace_in_files" => {
            let o = search::SearchOpts::from_json(req);
            wrap(search::replace_in_files(&need_path(req, "root")?, &o, s(req, "replacement"), &strings(&req["paths"])))
        }
        "fs_new_file" => wrap(fsops::new_file(&need_path(req, "path")?)),
        "fs_new_folder" => wrap(fsops::new_folder(&need_path(req, "path")?)),
        "fs_rename" => wrap(fsops::rename(&need_path(req, "from")?, &need_path(req, "to")?)),
        "fs_trash" => wrap(fsops::trash(&need_path(req, "path")?)),
        "reveal" => wrap(fsops::reveal(&need_path(req, "path")?)),
        "terminal" => wrap(fsops::terminal(&need_path(req, "path")?)),
        op => files::err(format!("Axentra Notepad doesn't know how to “{op}”.")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::tests::temp;

    #[test]
    fn requests_round_trip() {
        let data = temp("api-data");
        let dir = temp("api-files");
        let ctx = Ctx { data: data.clone(), primary: true, startup: json!({ "docs": [], "folders": [], "errors": [] }) };
        let init = handle(&ctx, &json!({ "op": "init" }));
        assert_eq!((init["ok"].as_bool(), init["primary"].as_bool(), init["session"].is_null()), (Some(true), Some(true), true));
        assert_eq!(init["encodings"][0], json!({ "id": "utf-8", "name": "UTF-8" }));
        let path = dir.join("a.txt").display().to_string();
        let saved = handle(&ctx, &json!({ "op": "save", "path": path, "text": "x\ny", "encoding": "utf-8", "eol": "crlf" }));
        assert_eq!(saved["ok"], true, "{saved}");
        assert_eq!(std::fs::read(&path).unwrap(), b"x\r\ny");
        let opened = handle(&ctx, &json!({ "op": "open", "path": path }));
        assert_eq!(opened["doc"]["text"], "x\ny");
        assert_eq!(handle(&ctx, &json!({ "op": "init" }))["recent"][0], std::fs::canonicalize(&path).unwrap().display().to_string());
        let missing = handle(&ctx, &json!({ "op": "open", "path": dir.join("new.txt") }));
        assert_eq!((missing["ok"].as_bool(), missing["notFound"].as_bool()), (Some(false), Some(true)));
        let resolved = handle(&ctx, &json!({ "op": "open_paths", "paths": [path, dir.join("new.txt"), dir] }));
        assert_eq!(resolved["docs"][1]["missing"], true);
        assert_eq!(resolved["folders"][0], std::fs::canonicalize(&dir).unwrap().display().to_string());
        let s = handle(&ctx, &json!({ "op": "session_save", "session": { "tabs": [] }, "buffers": { "t1": "hi" } }));
        assert_eq!(s["ok"], true, "{s}");
        let init = handle(&ctx, &json!({ "op": "init" }));
        assert_eq!((init["session"].clone(), init["buffers"]["t1"].clone()), (json!({ "tabs": [] }), json!("hi")));
        let secondary = Ctx { primary: false, ..ctx };
        assert!(handle(&secondary, &json!({ "op": "init" }))["session"].is_null());
        assert_eq!(handle(&secondary, &json!({ "op": "session_save", "session": {} }))["ignored"], true);
        assert_eq!(handle(&secondary, &json!({ "op": "nope" }))["ok"], false);
        assert_eq!(handle(&secondary, &json!({ "op": "list_dir", "path": dir }))["entries"][0]["name"], "a.txt");
        std::fs::remove_dir_all(data).ok();
        std::fs::remove_dir_all(dir).ok();
    }
}
