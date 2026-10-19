//! An in-memory OneDrive that speaks the subset of Microsoft Graph Axentra OneDrive
//! uses: delta feed (with paging and expiry), path addressing, simple and
//! session uploads, eTag preconditions, downloads via redirect, sharing
//! links, versions, and the OAuth token endpoint.
//!
//! Tests drive the "online" side through [`MockServer`]'s helper methods.

use std::collections::HashMap;
use std::sync::Arc;

use axentra_onedrive_graph::QuickXorHash;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde_json::{json, Value};

const ROOT: &str = "ROOT";

#[derive(Debug, Clone)]
struct MItem {
    id: String,
    name: String,
    parent: Option<String>,
    is_dir: bool,
    content: Vec<u8>,
    etag_n: u64,
    ctag_n: u64,
    mtime: DateTime<Utc>,
    changed: u64,
    versions: Vec<(String, Vec<u8>, DateTime<Utc>)>,
    shared: bool,
}

#[derive(Debug, Clone)]
struct Session {
    parent: String,
    name: String,
    existing: Option<String>,
    data: Vec<u8>,
    mtime: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
pub struct Drive {
    items: HashMap<String, MItem>,
    deleted: HashMap<String, (u64, Option<String>)>,
    seq: u64,
    next_id: u64,
    sessions: HashMap<String, Session>,
    /// Delta tokens older than this get HTTP 410.
    min_token: u64,
    /// Items per delta page (small values exercise paging).
    pub page_size: usize,
    /// "METHOD path" of every API request.
    pub log: Vec<String>,
    tokens_issued: u64,
    /// Fail the next N API requests with this status (e.g. 503 or 429).
    pub fail_next: Option<(u16, u32)>,
}

fn etag(it: &MItem) -> String {
    format!("\"{{{}}},{}\"", it.id, it.etag_n)
}

fn ctag(it: &MItem) -> String {
    format!("\"c:{{{}}},{}\"", it.id, it.ctag_n)
}

fn qxh(data: &[u8]) -> String {
    let mut h = QuickXorHash::new();
    h.update(data);
    h.base64()
}

impl Drive {
    fn new() -> Self {
        let mut d = Drive {
            page_size: 200,
            next_id: 1,
            ..Default::default()
        };
        d.seq = 1;
        d.items.insert(
            ROOT.into(),
            MItem {
                id: ROOT.into(),
                name: "root".into(),
                parent: None,
                is_dir: true,
                content: vec![],
                etag_n: 1,
                ctag_n: 1,
                mtime: Utc::now(),
                changed: 1,
                versions: vec![],
                shared: false,
            },
        );
        d
    }

    fn bump(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn touch(&mut self, id: &str, content_changed: bool) {
        let seq = self.bump();
        if let Some(it) = self.items.get_mut(id) {
            it.changed = seq;
            it.etag_n += 1;
            if content_changed {
                it.ctag_n += 1;
            }
        }
    }

    fn child_of(&self, parent: &str, name: &str) -> Option<&MItem> {
        self.items
            .values()
            .find(|i| i.parent.as_deref() == Some(parent) && i.name.eq_ignore_ascii_case(name))
    }

    fn by_path(&self, path: &str) -> Option<&MItem> {
        let mut cur = self.items.get(ROOT)?;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            cur = self.child_of(&cur.id.clone(), part)?;
        }
        Some(cur)
    }

    fn id_of(&self, path: &str) -> String {
        self.by_path(path)
            .unwrap_or_else(|| panic!("mock: no item at {path}"))
            .id
            .clone()
    }

    fn new_item(&mut self, parent: &str, name: &str, is_dir: bool, content: Vec<u8>) -> String {
        let id = format!("ITEM{:04}", self.next_id);
        self.next_id += 1;
        let seq = self.bump();
        self.items.insert(
            id.clone(),
            MItem {
                id: id.clone(),
                name: name.into(),
                parent: Some(parent.into()),
                is_dir,
                content: content.clone(),
                etag_n: 1,
                ctag_n: 1,
                mtime: Utc::now(),
                changed: seq,
                versions: if is_dir {
                    vec![]
                } else {
                    vec![("1.0".into(), content, Utc::now())]
                },
                shared: false,
            },
        );
        self.touch_parent(parent);
        id
    }

    fn touch_parent(&mut self, parent: &str) {
        if let Some(p) = self.items.get_mut(parent) {
            p.ctag_n += 1;
        }
    }

    fn set_content(&mut self, id: &str, data: Vec<u8>, mtime: Option<DateTime<Utc>>) {
        let seq = self.bump();
        if let Some(it) = self.items.get_mut(id) {
            it.content = data.clone();
            it.etag_n += 1;
            it.ctag_n += 1;
            it.changed = seq;
            it.mtime = mtime.unwrap_or_else(Utc::now);
            let v = format!("{}.0", it.versions.len() + 1);
            it.versions.push((v, data, Utc::now()));
        }
    }

    fn remove(&mut self, id: &str) {
        let children: Vec<String> = self
            .items
            .values()
            .filter(|i| i.parent.as_deref() == Some(id))
            .map(|i| i.id.clone())
            .collect();
        for c in children {
            self.remove(&c);
        }
        if let Some(it) = self.items.remove(id) {
            let seq = self.bump();
            self.deleted.insert(id.into(), (seq, it.parent));
        }
    }

    fn depth(&self, id: &str) -> usize {
        let mut d = 0;
        let mut cur = self.items.get(id).and_then(|i| i.parent.clone());
        while let Some(p) = cur {
            d += 1;
            cur = self.items.get(&p).and_then(|i| i.parent.clone());
        }
        d
    }

    fn json(&self, it: &MItem, base: &str) -> Value {
        let mut v = json!({
            "id": it.id,
            "name": it.name,
            "eTag": etag(it),
            "cTag": ctag(it),
            "createdDateTime": it.mtime.to_rfc3339(),
            "lastModifiedDateTime": it.mtime.to_rfc3339(),
            "fileSystemInfo": { "lastModifiedDateTime": it.mtime.to_rfc3339() },
            "webUrl": format!("https://onedrive.mock/view?id={}", it.id),
            "size": if it.is_dir { 0 } else { it.content.len() },
        });
        if let Some(p) = &it.parent {
            v["parentReference"] = json!({ "id": p, "driveId": "DRIVE1" });
        } else {
            v["root"] = json!({});
        }
        if it.is_dir {
            let n = self
                .items
                .values()
                .filter(|c| c.parent.as_deref() == Some(&it.id))
                .count();
            v["folder"] = json!({ "childCount": n });
        } else {
            v["file"] = json!({ "mimeType": "application/octet-stream", "hashes": { "quickXorHash": qxh(&it.content) } });
            v["@microsoft.graph.downloadUrl"] = json!(format!("{base}/download/{}", it.id));
        }
        if it.shared {
            v["shared"] = json!({ "scope": "anonymous" });
        }
        v
    }

    fn used(&self) -> u64 {
        self.items.values().map(|i| i.content.len() as u64).sum()
    }
}

#[derive(Clone)]
struct AppState {
    drive: Arc<Mutex<Drive>>,
    origin: Arc<String>,
}

pub struct MockServer {
    pub origin: String,
    pub graph_base: String,
    drive: Arc<Mutex<Drive>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MockServer {
    pub async fn start() -> Self {
        Self::start_on("127.0.0.1:0").await
    }

    pub async fn start_on(addr: &str) -> Self {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("bind mock");
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let drive = Arc::new(Mutex::new(Drive::new()));
        let state = AppState {
            drive: drive.clone(),
            origin: Arc::new(origin.clone()),
        };
        let app = Router::new()
            .fallback(handle)
            .layer(DefaultBodyLimit::disable())
            .with_state(state);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            graph_base: format!("{origin}/v1.0"),
            origin,
            drive,
            task,
        }
    }

    /// Token file contents for a signed-in client of this mock.
    pub fn tokens_json(&self, expired: bool) -> String {
        let expires_at = if expired {
            0
        } else {
            Utc::now().timestamp() + 3600
        };
        json!({ "access_token": "mock-access-0", "refresh_token": "mock-refresh", "expires_at": expires_at }).to_string()
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut Drive) -> R) -> R {
        f(&mut self.drive.lock())
    }

    // ----- "online" changes, as if made on the web or another PC -------------

    pub fn add_folder(&self, parent_path: &str, name: &str) -> String {
        self.with(|d| {
            let p = d.id_of(parent_path);
            d.new_item(&p, name, true, vec![])
        })
    }

    pub fn add_file(&self, parent_path: &str, name: &str, content: &[u8]) -> String {
        self.with(|d| {
            let p = d.id_of(parent_path);
            d.new_item(&p, name, false, content.to_vec())
        })
    }

    pub fn modify(&self, path: &str, content: &[u8]) {
        self.with(|d| {
            let id = d.id_of(path);
            d.set_content(&id, content.to_vec(), None)
        })
    }

    pub fn rename(&self, path: &str, new_name: &str) {
        self.with(|d| {
            let id = d.id_of(path);
            d.items.get_mut(&id).unwrap().name = new_name.into();
            d.touch(&id, false);
        })
    }

    pub fn move_to(&self, path: &str, new_parent_path: &str) {
        self.with(|d| {
            let id = d.id_of(path);
            let p = d.id_of(new_parent_path);
            d.items.get_mut(&id).unwrap().parent = Some(p);
            d.touch(&id, false);
        })
    }

    pub fn delete(&self, path: &str) {
        self.with(|d| {
            let id = d.id_of(path);
            d.remove(&id)
        })
    }

    pub fn content(&self, path: &str) -> Option<Vec<u8>> {
        self.with(|d| d.by_path(path).map(|i| i.content.clone()))
    }

    pub fn exists(&self, path: &str) -> bool {
        self.with(|d| d.by_path(path).is_some())
    }

    pub fn id(&self, path: &str) -> Option<String> {
        self.with(|d| d.by_path(path).map(|i| i.id.clone()))
    }

    pub fn version_count(&self, path: &str) -> usize {
        self.with(|d| d.by_path(path).map(|i| i.versions.len()).unwrap_or(0))
    }

    /// Names directly inside a folder, sorted.
    pub fn list(&self, path: &str) -> Vec<String> {
        self.with(|d| {
            let Some(dir) = d.by_path(path) else {
                return vec![];
            };
            let id = dir.id.clone();
            let mut v: Vec<String> = d
                .items
                .values()
                .filter(|i| i.parent.as_deref() == Some(&id))
                .map(|i| i.name.clone())
                .collect();
            v.sort();
            v
        })
    }

    pub fn expire_delta_tokens(&self) {
        self.with(|d| d.min_token = d.seq + 1);
    }

    pub fn requests(&self, needle: &str) -> usize {
        self.with(|d| d.log.iter().filter(|l| l.contains(needle)).count())
    }
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        axum::Json(json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

fn ok_json(v: Value) -> Response {
    (StatusCode::OK, axum::Json(v)).into_response()
}

fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

fn query(uri: &Uri) -> HashMap<String, String> {
    uri.query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_default()
}

/// Splits `items/<ref>[/<action>...]`, where `<ref>` is either an ID or the
/// path form `<parent-id>:/<name>:`.
enum Target {
    Id(String),
    Child { parent: String, name: String },
}

fn parse_items_path(rest: &str) -> (Target, Vec<String>) {
    if let Some(colon) = rest.find(":/") {
        let parent = rest[..colon].to_string();
        let after = &rest[colon + 2..];
        let (name, tail) = match after.find(':') {
            Some(end) => (&after[..end], after[end + 1..].trim_start_matches('/')),
            None => (after, ""),
        };
        let tail = tail
            .split('/')
            .filter(|s| !s.is_empty())
            .map(decode)
            .collect();
        return (
            Target::Child {
                parent,
                name: decode(name),
            },
            tail,
        );
    }
    let mut parts = rest.split('/').filter(|s| !s.is_empty());
    let id = decode(parts.next().unwrap_or(""));
    (Target::Id(id), parts.map(decode).collect())
}

async fn handle(
    State(st): State<AppState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().to_string();
    let base = st.origin.as_str();

    if path.ends_with("/oauth2/v2.0/token") {
        return token(&st, &body);
    }
    if let Some(sid) = path.strip_prefix("/upload/") {
        return upload_fragment(&st, sid, &headers, body);
    }
    if let Some(id) = path.strip_prefix("/download/") {
        let d = st.drive.lock();
        return match d.items.get(id) {
            Some(it) => (StatusCode::OK, it.content.clone()).into_response(),
            None => error(StatusCode::NOT_FOUND, "itemNotFound", "gone"),
        };
    }
    let Some(api) = path.strip_prefix("/v1.0") else {
        return error(StatusCode::NOT_FOUND, "notFound", "unknown endpoint");
    };

    {
        let mut d = st.drive.lock();
        d.log.push(format!("{method} {api}"));
        if let Some((status, n)) = d.fail_next {
            d.fail_next = if n > 1 { Some((status, n - 1)) } else { None };
            let mut r = error(
                StatusCode::from_u16(status).unwrap(),
                "injected",
                "injected failure",
            );
            r.headers_mut().insert("retry-after", "0".parse().unwrap());
            return r;
        }
    }
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !auth.starts_with("Bearer mock-access-") {
        return error(
            StatusCode::UNAUTHORIZED,
            "InvalidAuthenticationToken",
            "no token",
        );
    }

    let mut d = st.drive.lock();
    match (method.clone(), api) {
        (Method::GET, "/me") => ok_json(
            json!({ "id": "USER1", "displayName": "Test User", "mail": "test.user@example.com" }),
        ),
        (Method::GET, "/me/drive") => {
            let used = d.used();
            let total: u64 = 5 * 1024 * 1024 * 1024;
            ok_json(json!({
                "id": "DRIVE1", "driveType": "personal",
                "owner": { "user": { "displayName": "Test User" } },
                "webUrl": "https://onedrive.mock/",
                "quota": { "total": total, "used": used, "remaining": total - used, "deleted": 0, "state": "normal" }
            }))
        }
        (Method::GET, "/me/drive/root/delta") => delta(&d, &uri, base),
        _ => {
            let Some(rest) = api.strip_prefix("/me/drive/items/") else {
                return error(StatusCode::NOT_FOUND, "notFound", "unknown endpoint");
            };
            let (target, tail) = parse_items_path(rest);
            items(&mut d, method, target, tail, &uri, &headers, body, base)
        }
    }
}

fn token(st: &AppState, body: &[u8]) -> Response {
    let form: HashMap<String, String> = url::form_urlencoded::parse(body).into_owned().collect();
    if form.get("refresh_token").map(String::as_str) == Some("revoked") {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({ "error": "invalid_grant", "error_description": "revoked" })),
        )
            .into_response();
    }
    let mut d = st.drive.lock();
    d.tokens_issued += 1;
    ok_json(json!({
        "token_type": "Bearer",
        "access_token": format!("mock-access-{}", d.tokens_issued),
        "refresh_token": "mock-refresh",
        "expires_in": 3600,
    }))
}

fn delta(d: &Drive, uri: &Uri, base: &str) -> Response {
    let q = query(uri);
    let since: u64 = q.get("token").and_then(|t| t.parse().ok()).unwrap_or(0);
    if q.contains_key("token") && since < d.min_token {
        return error(StatusCode::GONE, "resyncRequired", "token expired");
    }
    let end: u64 = q.get("end").and_then(|t| t.parse().ok()).unwrap_or(d.seq);
    let skip: usize = q.get("skip").and_then(|t| t.parse().ok()).unwrap_or(0);

    let mut changed: Vec<&MItem> = d
        .items
        .values()
        .filter(|i| since == 0 || (i.changed > since && i.changed <= end))
        .collect();
    // Parents before children, then in change order.
    changed.sort_by_key(|i| (d.depth(&i.id), i.changed, i.id.clone()));
    let mut all: Vec<Value> = changed.iter().map(|i| d.json(i, base)).collect();
    if since > 0 {
        let mut gone: Vec<(&String, &(u64, Option<String>))> = d
            .deleted
            .iter()
            .filter(|(_, (seq, _))| *seq > since && *seq <= end)
            .collect();
        gone.sort_by_key(|(_, (seq, _))| *seq);
        for (id, (_, parent)) in gone {
            all.push(json!({ "id": id, "deleted": { "state": "deleted" }, "parentReference": { "id": parent } }));
        }
    }
    let page: Vec<Value> = all.iter().skip(skip).take(d.page_size).cloned().collect();
    let mut resp = json!({ "value": page });
    if skip + d.page_size < all.len() {
        let token = if since > 0 {
            format!("token={since}&")
        } else {
            String::new()
        };
        resp["@odata.nextLink"] = json!(format!(
            "{base}/v1.0/me/drive/root/delta?{token}end={end}&skip={}",
            skip + d.page_size
        ));
    } else {
        resp["@odata.deltaLink"] = json!(format!("{base}/v1.0/me/drive/root/delta?token={end}"));
    }
    ok_json(resp)
}

fn precondition(it: &MItem, headers: &HeaderMap) -> Option<Response> {
    let want = headers.get("if-match")?.to_str().ok()?;
    (want != etag(it)).then(|| {
        error(
            StatusCode::PRECONDITION_FAILED,
            "preconditionFailed",
            "eTag mismatch",
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn items(
    d: &mut Drive,
    method: Method,
    target: Target,
    tail: Vec<String>,
    uri: &Uri,
    headers: &HeaderMap,
    body: Bytes,
    base: &str,
) -> Response {
    let tail: Vec<&str> = tail.iter().map(String::as_str).collect();
    let q = query(uri);
    match target {
        Target::Child { parent, name } => {
            if !d.items.contains_key(&parent) {
                return error(StatusCode::NOT_FOUND, "itemNotFound", "no parent");
            }
            let existing = d.child_of(&parent, &name).map(|i| i.id.clone());
            match (method, tail.as_slice()) {
                (Method::GET, []) => match existing {
                    Some(id) => ok_json(d.json(&d.items[&id], base)),
                    None => error(StatusCode::NOT_FOUND, "itemNotFound", "no such child"),
                },
                (Method::PUT, ["content"]) => {
                    let fail = q
                        .get("@microsoft.graph.conflictBehavior")
                        .map(String::as_str)
                        == Some("fail");
                    let id = match existing {
                        Some(_) if fail => {
                            return error(StatusCode::CONFLICT, "nameAlreadyExists", "name taken")
                        }
                        Some(id) => {
                            d.set_content(&id, body.to_vec(), None);
                            id
                        }
                        None => d.new_item(&parent, &name, false, body.to_vec()),
                    };
                    (StatusCode::CREATED, axum::Json(d.json(&d.items[&id], base))).into_response()
                }
                (Method::POST, ["createUploadSession"]) => {
                    let spec: Value = serde_json::from_slice(&body).unwrap_or_default();
                    let behavior = spec["item"]["@microsoft.graph.conflictBehavior"]
                        .as_str()
                        .unwrap_or("fail");
                    if existing.is_some() && behavior == "fail" {
                        return error(StatusCode::CONFLICT, "nameAlreadyExists", "name taken");
                    }
                    new_session(d, parent, name, existing, &spec, base)
                }
                _ => error(StatusCode::METHOD_NOT_ALLOWED, "notSupported", "mock"),
            }
        }
        Target::Id(id) => {
            let Some(it) = d.items.get(&id).cloned() else {
                return error(StatusCode::NOT_FOUND, "itemNotFound", "no such item");
            };
            match (method, tail.as_slice()) {
                (Method::GET, []) => ok_json(d.json(&it, base)),
                (Method::PATCH, []) => {
                    if let Some(r) = precondition(&it, headers) {
                        return r;
                    }
                    let spec: Value = serde_json::from_slice(&body).unwrap_or_default();
                    let new_parent = spec["parentReference"]["id"].as_str().map(str::to_string);
                    let new_name = spec["name"].as_str().map(str::to_string);
                    let target_parent =
                        new_parent.clone().or(it.parent.clone()).unwrap_or_default();
                    let target_name = new_name.clone().unwrap_or(it.name.clone());
                    if let Some(other) = d.child_of(&target_parent, &target_name) {
                        if other.id != id {
                            return error(StatusCode::CONFLICT, "nameAlreadyExists", "name taken");
                        }
                    }
                    let m = spec["fileSystemInfo"]["lastModifiedDateTime"]
                        .as_str()
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|t| t.with_timezone(&Utc));
                    {
                        let e = d.items.get_mut(&id).unwrap();
                        if let Some(n) = new_name {
                            e.name = n;
                        }
                        if let Some(p) = new_parent {
                            e.parent = Some(p);
                        }
                        if let Some(m) = m {
                            e.mtime = m;
                        }
                    }
                    d.touch(&id, false);
                    ok_json(d.json(&d.items[&id], base))
                }
                (Method::DELETE, []) => {
                    if let Some(r) = precondition(&it, headers) {
                        return r;
                    }
                    d.remove(&id);
                    StatusCode::NO_CONTENT.into_response()
                }
                (Method::GET, ["content"]) => {
                    // Like Graph: a redirect to a pre-authenticated URL.
                    let mut r = StatusCode::FOUND.into_response();
                    r.headers_mut()
                        .insert("location", format!("{base}/download/{id}").parse().unwrap());
                    r
                }
                (Method::PUT, ["content"]) => {
                    if let Some(r) = precondition(&it, headers) {
                        return r;
                    }
                    d.set_content(&id, body.to_vec(), None);
                    ok_json(d.json(&d.items[&id], base))
                }
                (Method::POST, ["children"]) => {
                    let spec: Value = serde_json::from_slice(&body).unwrap_or_default();
                    let name = spec["name"].as_str().unwrap_or("").to_string();
                    if d.child_of(&id, &name).is_some() {
                        return error(StatusCode::CONFLICT, "nameAlreadyExists", "name taken");
                    }
                    let new = d.new_item(&id, &name, true, vec![]);
                    (
                        StatusCode::CREATED,
                        axum::Json(d.json(&d.items[&new], base)),
                    )
                        .into_response()
                }
                (Method::POST, ["createUploadSession"]) => {
                    if let Some(r) = precondition(&it, headers) {
                        return r;
                    }
                    let spec: Value = serde_json::from_slice(&body).unwrap_or_default();
                    let parent = it.parent.clone().unwrap_or_default();
                    new_session(d, parent, it.name.clone(), Some(id), &spec, base)
                }
                (Method::POST, ["createLink"]) => {
                    d.items.get_mut(&id).unwrap().shared = true;
                    d.touch(&id, false);
                    ok_json(
                        json!({ "id": "PERM1", "link": { "type": "view", "scope": "anonymous",
                        "webUrl": format!("https://1drv.ms/mock/{id}") } }),
                    )
                }
                (Method::GET, ["versions"]) => {
                    let v: Vec<Value> = it
                        .versions
                        .iter()
                        .rev()
                        .map(|(vid, data, t)| json!({ "id": vid, "size": data.len(), "lastModifiedDateTime": t.to_rfc3339(),
                            "lastModifiedBy": { "user": { "displayName": "Test User" } } }))
                        .collect();
                    ok_json(json!({ "value": v }))
                }
                (Method::POST, ["versions", vid, "restoreVersion"]) => {
                    let Some((_, data, _)) = it.versions.iter().find(|(v, _, _)| v == vid).cloned()
                    else {
                        return error(StatusCode::NOT_FOUND, "itemNotFound", "no such version");
                    };
                    d.set_content(&id, data, None);
                    StatusCode::NO_CONTENT.into_response()
                }
                _ => error(StatusCode::METHOD_NOT_ALLOWED, "notSupported", "mock"),
            }
        }
    }
}

fn new_session(
    d: &mut Drive,
    parent: String,
    name: String,
    existing: Option<String>,
    spec: &Value,
    base: &str,
) -> Response {
    let sid = format!("S{}", d.sessions.len() + 1);
    let mtime = spec["item"]["fileSystemInfo"]["lastModifiedDateTime"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc));
    d.sessions.insert(
        sid.clone(),
        Session {
            parent,
            name,
            existing,
            data: vec![],
            mtime,
        },
    );
    ok_json(
        json!({ "uploadUrl": format!("{base}/upload/{sid}"), "expirationDateTime": Utc::now().to_rfc3339() }),
    )
}

fn upload_fragment(st: &AppState, sid: &str, headers: &HeaderMap, body: Bytes) -> Response {
    if headers.contains_key("authorization") {
        return error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "upload URLs take no token",
        );
    }
    let range = headers
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    // "bytes START-END/TOTAL"
    let parsed = range
        .strip_prefix("bytes ")
        .and_then(|r| r.split_once('/'))
        .and_then(|(span, total)| {
            let (a, b) = span.split_once('-')?;
            Some((
                a.parse::<u64>().ok()?,
                b.parse::<u64>().ok()?,
                total.parse::<u64>().ok()?,
            ))
        });
    let Some((start, end, total)) = parsed else {
        return error(StatusCode::BAD_REQUEST, "invalidRange", range);
    };
    let mut d = st.drive.lock();
    let Some(s) = d.sessions.get_mut(sid) else {
        return error(StatusCode::NOT_FOUND, "itemNotFound", "no session");
    };
    if start != s.data.len() as u64 || end + 1 - start != body.len() as u64 {
        return error(
            StatusCode::RANGE_NOT_SATISFIABLE,
            "invalidRange",
            "out of order",
        );
    }
    s.data.extend_from_slice(&body);
    if (s.data.len() as u64) < total {
        let next = s.data.len();
        return (
            StatusCode::ACCEPTED,
            axum::Json(json!({ "nextExpectedRanges": [format!("{next}-")] })),
        )
            .into_response();
    }
    let s = d.sessions.remove(sid).unwrap();
    let id = match s.existing {
        Some(id) if d.items.contains_key(&id) => {
            d.set_content(&id, s.data, s.mtime);
            id
        }
        _ => match d.child_of(&s.parent, &s.name).map(|i| i.id.clone()) {
            Some(id) => {
                d.set_content(&id, s.data, s.mtime);
                id
            }
            None => {
                let id = d.new_item(&s.parent, &s.name, false, s.data);
                if let Some(m) = s.mtime {
                    d.items.get_mut(&id).unwrap().mtime = m;
                }
                id
            }
        },
    };
    let base = st.origin.as_str();
    (StatusCode::CREATED, axum::Json(d.json(&d.items[&id], base))).into_response()
}
