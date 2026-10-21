//! HTTP client for the OneDrive endpoints of Microsoft Graph.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::auth::TokenStore;
use crate::quickxor::QuickXorHash;
use crate::throttle::RateLimiter;
use crate::types::*;

/// Files up to this size go up in a single PUT; larger ones use an upload session.
pub const SIMPLE_UPLOAD_MAX: u64 = 4 * 1024 * 1024;
/// Upload-session fragments must be multiples of 320 KiB.
const FRAGMENT_UNIT: u64 = 320 * 1024;
const MAX_ATTEMPTS: u32 = 6;
/// Upload bodies go to the connection in pieces this big, so that a stalled
/// upload can be told apart from a slow one.
const PIECE: usize = 64 * 1024;
/// An upload gives up once its body has not moved, or no reply has come,
/// for this long.
const STALL_TIMEOUT: Duration = Duration::from_secs(120);

const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn enc(name: &str) -> String {
    utf8_percent_encode(name, PATH_SEGMENT).to_string()
}

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("not signed in")]
    NotSignedIn,
    #[error("{message} (HTTP {status} {code})")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error("network error: {0}")]
    Network(#[source] reqwest::Error),
    #[error("network error: the upload stopped moving")]
    Stalled,
    /// Written to while it was being sent: that version goes up instead.
    #[error("the file changed while it was uploading")]
    Changed,
    #[error("{0}")]
    Other(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type GResult<T> = Result<T, GraphError>;

impl GraphError {
    pub fn status(&self) -> Option<u16> {
        match self {
            GraphError::Api { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn code(&self) -> Option<&str> {
        match self {
            GraphError::Api { code, .. } => Some(code),
            _ => None,
        }
    }

    pub fn is_not_found(&self) -> bool {
        self.status() == Some(404)
    }

    /// No route to Microsoft at all (as opposed to an API error).
    pub fn is_offline(&self) -> bool {
        match self {
            GraphError::Network(e) => e.is_connect() || e.is_timeout() || e.is_request(),
            GraphError::Stalled => true,
            _ => false,
        }
    }

    /// A network failure that sending the same request again may get past.
    fn is_transient(&self) -> bool {
        match self {
            GraphError::Network(e) => e.is_connect() || e.is_timeout(),
            GraphError::Stalled => true,
            _ => false,
        }
    }

    /// Leaves an upload session usable for a later attempt: no network,
    /// throttling or a server fault.
    fn keeps_session(&self) -> bool {
        self.is_offline() || matches!(self.status(), Some(429 | 500 | 502 | 503 | 504))
    }
}

impl From<reqwest::Error> for GraphError {
    fn from(e: reqwest::Error) -> Self {
        GraphError::Network(e)
    }
}

impl From<serde_json::Error> for GraphError {
    fn from(e: serde_json::Error) -> Self {
        GraphError::Other(format!("unexpected response: {e}"))
    }
}

async fn api_error(resp: Response) -> GraphError {
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let parsed: Option<serde_json::Value> = serde_json::from_str(&body).ok();
    let err = parsed.as_ref().and_then(|v| v.get("error"));
    let code = err
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_str())
        .unwrap_or("unknown")
        .to_string();
    let message = err
        .and_then(|e| e.get("message"))
        .and_then(|c| c.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if body.is_empty() {
                format!("HTTP {status}")
            } else {
                body.clone()
            }
        });
    GraphError::Api {
        status,
        code,
        message,
    }
}

fn retry_after(resp: &Response) -> Option<Duration> {
    let secs: u64 = resp
        .headers()
        .get("retry-after")?
        .to_str()
        .ok()?
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs.min(300)))
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(500 * 2u64.pow(attempt.min(6)))
}

fn retryable(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
}

/// Send `rb` with `data` as its body, giving up only once the body stops
/// moving, or no reply comes after it, for `stall`. reqwest's read timeout
/// runs from the start of the request, so it would cut off any body that
/// takes longer to send, which on a slow uplink is most uploads.
async fn send_watched(rb: RequestBuilder, data: Bytes, stall: Duration) -> GResult<Response> {
    let start = tokio::time::Instant::now();
    // When the connection last took a piece, in milliseconds after `start`.
    let moved = Arc::new(AtomicU64::new(0));
    let mark = moved.clone();
    let len = data.len();
    let pieces = futures_util::stream::iter((0..len).step_by(PIECE).map(move |at| {
        mark.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
        Ok::<_, std::convert::Infallible>(data.slice(at..(at + PIECE).min(len)))
    }));
    let sent = rb
        .header(reqwest::header::CONTENT_LENGTH, len)
        .body(reqwest::Body::wrap_stream(pieces))
        .send();
    tokio::pin!(sent);
    loop {
        let last = Duration::from_millis(moved.load(Ordering::Relaxed));
        let idle = start.elapsed().saturating_sub(last);
        if idle >= stall {
            return Err(GraphError::Stalled);
        }
        tokio::select! {
            r = &mut sent => return Ok(r?),
            () = tokio::time::sleep(stall - idle) => {}
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeltaPage {
    pub items: Vec<DriveItem>,
    pub next_link: Option<String>,
    pub delta_link: Option<String>,
}

/// Where an upload lands.
#[derive(Debug, Clone, Copy)]
pub enum UploadTarget<'a> {
    /// A new version of an existing item.
    Existing(&'a str),
    /// A new file. Fails with HTTP 409 if the name is already taken.
    New { parent_id: &'a str, name: &'a str },
}

impl UploadTarget<'_> {
    /// Names the target, to find an upload session left for it.
    fn key(&self) -> String {
        match self {
            UploadTarget::Existing(id) => id.to_string(),
            UploadTarget::New { parent_id, name } => format!("{parent_id}/{name}"),
        }
    }
}

/// An upload session being filled. One cut short by a network failure, or
/// by pausing, is kept, so the next attempt at the same file carries on
/// where it stopped instead of sending everything again, as the Windows
/// client does.
struct OpenSession {
    upload_url: String,
    expires: Option<DateTime<Utc>>,
    stamp: Stamp,
    /// The last fragment went up, so the upload may have finished.
    sent_final: bool,
    /// QuickXorHash of the bytes sent so far from the start, and how many.
    hashed: Option<(QuickXorHash, u64)>,
}

impl OpenSession {
    /// Count `buf`, read from `at`, into the hash of what was sent.
    fn hash(&mut self, at: u64, buf: &[u8]) {
        let end = at + buf.len() as u64;
        self.hashed = match self.hashed.take() {
            // Sent again after a failure: the file hasn't changed (see
            // `Stamp::same_file`), so only bytes past the hashed ones count.
            Some((mut h, n)) if (at..end).contains(&n) => {
                h.update(&buf[(n - at) as usize..]);
                Some((h, end))
            }
            Some((h, n)) if n >= end => Some((h, n)),
            _ => None,
        };
    }

    /// The hash of the whole file, once every byte of it went up.
    fn sent_hash(&self) -> Option<String> {
        let (h, n) = self.hashed.as_ref()?;
        (self.sent_final && *n == self.stamp.len).then(|| h.base64())
    }
}

/// An upload session in use. If the upload is cancelled (syncing paused,
/// say) this puts it back for the next attempt, as a network failure does.
struct HeldSession<'a> {
    graph: &'a Graph,
    key: &'a str,
    session: Option<OpenSession>,
}

impl HeldSession<'_> {
    fn get(&mut self) -> &mut OpenSession {
        self.session.as_mut().expect("held session")
    }

    /// No longer to be kept: finished, gone, or cancelled here.
    fn take(&mut self) -> OpenSession {
        self.session.take().expect("held session")
    }
}

impl Drop for HeldSession<'_> {
    fn drop(&mut self) {
        if let Some(s) = self.session.take() {
            self.graph
                .sessions
                .lock()
                .unwrap()
                .insert(self.key.to_string(), s);
        }
    }
}

/// What became of an item since a kept upload session was made for it.
enum Online {
    /// Nothing: the session can carry on.
    Unchanged,
    /// It has the file's content: the last fragment landed, its reply lost.
    Uploaded(Box<DriveItem>),
    /// Edited or deleted: a new session gets the 412 or 404.
    Changed,
}

/// The local file and request an upload session was made for. The change
/// time moves on every write, even one that keeps the size and mtime.
#[derive(Debug, Clone, PartialEq)]
struct Stamp {
    ino: u64,
    len: u64,
    ctime: (i64, i64),
    mtime: Option<DateTime<Utc>>,
    if_match: Option<String>,
}

impl Stamp {
    fn new(meta: &std::fs::Metadata, mtime: Option<DateTime<Utc>>, if_match: Option<&str>) -> Self {
        Self {
            ino: meta.ino(),
            len: meta.len(),
            ctime: (meta.ctime(), meta.ctime_nsec()),
            mtime,
            if_match: if_match.map(str::to_string),
        }
    }

    /// Whether the file is still as it was when the stamp was made.
    fn same_file(&self, meta: &std::fs::Metadata) -> bool {
        (self.ino, self.len, self.ctime)
            == (meta.ino(), meta.len(), (meta.ctime(), meta.ctime_nsec()))
    }
}

pub struct Graph {
    http: reqwest::Client,
    /// For upload bodies: no read timeout, see [`send_watched`].
    upload_http: reqwest::Client,
    base: RwLock<String>,
    tokens: Arc<TokenStore>,
    /// Interrupted upload sessions by [`UploadTarget::key`].
    sessions: Mutex<HashMap<String, OpenSession>>,
    /// QuickXorHashes of what uploads sent to each target since one last
    /// went through and was recorded ([`Graph::forget_sent`]), by
    /// [`UploadTarget::key`]: see [`Graph::sent_before`].
    sent: Mutex<HashMap<String, Vec<String>>>,
    pub upload_limit: RateLimiter,
    pub download_limit: RateLimiter,
}

impl Graph {
    pub fn new(base: &str, tokens: Arc<TokenStore>) -> Self {
        let builder = || {
            reqwest::Client::builder()
                .user_agent(concat!(
                    "AxentraOneDrive/",
                    env!("CARGO_PKG_VERSION"),
                    " (Linux)"
                ))
                .connect_timeout(Duration::from_secs(20))
        };
        let http = builder()
            .read_timeout(Duration::from_secs(120))
            .build()
            .expect("building HTTP client");
        let upload_http = builder().build().expect("building HTTP client");
        Self {
            http,
            upload_http,
            base: RwLock::new(base.trim_end_matches('/').to_string()),
            tokens,
            sessions: Mutex::new(HashMap::new()),
            sent: Mutex::new(HashMap::new()),
            upload_limit: RateLimiter::new(0),
            download_limit: RateLimiter::new(0),
        }
    }

    pub fn set_base(&self, base: &str) {
        *self.base.write().unwrap() = base.trim_end_matches('/').to_string();
    }

    pub fn tokens(&self) -> &Arc<TokenStore> {
        &self.tokens
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base.read().unwrap(), path)
    }

    /// Send an authenticated request, refreshing tokens on 401 and backing
    /// off on throttling or transient server errors.
    async fn send(&self, build: impl Fn(&reqwest::Client) -> RequestBuilder) -> GResult<Response> {
        self.send_with(build, None).await
    }

    /// [`send`](Self::send) with `upload` as the request body, if given,
    /// watched for stalls rather than timed out (see [`send_watched`]).
    async fn send_with(
        &self,
        build: impl Fn(&reqwest::Client) -> RequestBuilder,
        upload: Option<&Bytes>,
    ) -> GResult<Response> {
        let mut attempt = 0;
        let mut refreshed = false;
        loop {
            let token = self.tokens.access_token().await?;
            let sent = match upload {
                Some(data) => {
                    let rb = build(&self.upload_http).bearer_auth(&token);
                    send_watched(rb, data.clone(), STALL_TIMEOUT).await
                }
                None => build(&self.http)
                    .bearer_auth(&token)
                    .send()
                    .await
                    .map_err(GraphError::from),
            };
            match sent {
                Err(e) if attempt + 1 < MAX_ATTEMPTS && e.is_transient() => {
                    tokio::time::sleep(backoff(attempt)).await;
                    attempt += 1;
                }
                Err(e) => return Err(e),
                Ok(resp) if resp.status() == StatusCode::UNAUTHORIZED && !refreshed => {
                    self.tokens.invalidate(&token).await;
                    refreshed = true;
                }
                Ok(resp) if retryable(resp.status()) && attempt + 1 < MAX_ATTEMPTS => {
                    let wait = retry_after(&resp).unwrap_or_else(|| backoff(attempt));
                    tracing::debug!(status = %resp.status(), ?wait, "Graph asked us to back off");
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
                Ok(resp) if resp.status().is_success() => return Ok(resp),
                Ok(resp) => return Err(api_error(resp).await),
            }
        }
    }

    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> GResult<T> {
        let resp = self.send(|c| c.get(url)).await?;
        Ok(resp.json().await?)
    }

    async fn send_json<T: DeserializeOwned>(
        &self,
        method: Method,
        url: &str,
        body: &serde_json::Value,
    ) -> GResult<T> {
        let resp = self
            .send(|c| c.request(method.clone(), url).json(body))
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn me(&self) -> GResult<User> {
        self.get_json(&self.url("/me")).await
    }

    pub async fn drive(&self) -> GResult<Drive> {
        self.get_json(&self.url("/me/drive")).await
    }

    pub async fn item(&self, id: &str) -> GResult<DriveItem> {
        self.get_json(&self.url(&format!("/me/drive/items/{id}")))
            .await
    }

    /// Look up a child by name, `None` if there is no such child.
    pub async fn child(&self, parent_id: &str, name: &str) -> GResult<Option<DriveItem>> {
        let url = self.url(&format!("/me/drive/items/{parent_id}:/{}:", enc(name)));
        match self.get_json(&url).await {
            Ok(item) => Ok(Some(item)),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// One page of the change feed. Pass `None` to start a full enumeration,
    /// otherwise the `next_link` or saved `delta_link` from a previous page.
    /// A 410 error means the saved link is stale and a full resync is needed.
    pub async fn delta_page(&self, link: Option<&str>) -> GResult<DeltaPage> {
        let url = match link {
            Some(l) => l.to_string(),
            None => self.url("/me/drive/root/delta"),
        };
        let page: Collection<DriveItem> = self.get_json(&url).await?;
        Ok(DeltaPage {
            items: page.value,
            next_link: page.next_link,
            delta_link: page.delta_link,
        })
    }

    pub async fn create_folder(&self, parent_id: &str, name: &str) -> GResult<DriveItem> {
        let url = self.url(&format!("/me/drive/items/{parent_id}/children"));
        let body = json!({
            "name": name,
            "folder": {},
            "@microsoft.graph.conflictBehavior": "fail",
        });
        self.send_json(Method::POST, &url, &body).await
    }

    /// Rename and/or move an item.
    pub async fn update(
        &self,
        id: &str,
        name: Option<&str>,
        parent_id: Option<&str>,
    ) -> GResult<DriveItem> {
        let mut body = json!({});
        if let Some(n) = name {
            body["name"] = json!(n);
        }
        if let Some(p) = parent_id {
            body["parentReference"] = json!({ "id": p });
        }
        let url = self.url(&format!("/me/drive/items/{id}"));
        self.send_json(Method::PATCH, &url, &body).await
    }

    /// Set an item's modification time. With `if_match`, only while the
    /// item still has that eTag (HTTP 412 otherwise).
    ///
    /// Without it, whatever is online then gets the date: the item that
    /// comes back may hold content saved over the one the caller meant.
    pub async fn set_modified(
        &self,
        id: &str,
        mtime: DateTime<Utc>,
        if_match: Option<&str>,
    ) -> GResult<DriveItem> {
        let url = self.url(&format!("/me/drive/items/{id}"));
        let body = json!({ "fileSystemInfo": { "lastModifiedDateTime": mtime.to_rfc3339() } });
        let resp = self
            .send(|c| {
                let rb = c.patch(&url).json(&body);
                match if_match {
                    Some(tag) => rb.header("If-Match", tag),
                    None => rb,
                }
            })
            .await?;
        Ok(resp.json().await?)
    }

    /// Delete (moves the item to the OneDrive recycle bin). Already-gone is fine.
    pub async fn delete(&self, id: &str) -> GResult<()> {
        let url = self.url(&format!("/me/drive/items/{id}"));
        match self.send(|c| c.delete(&url)).await {
            Ok(_) => Ok(()),
            Err(e) if e.is_not_found() => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Stream an item's content to `dest`, calling `progress` with bytes so far.
    pub async fn download(
        &self,
        id: &str,
        dest: &Path,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> GResult<u64> {
        let url = self.url(&format!("/me/drive/items/{id}/content"));
        let resp = self.send(|c| c.get(&url)).await?;
        let mut file = tokio::fs::File::create(dest).await?;
        let mut stream = resp.bytes_stream();
        let mut done = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            done += chunk.len() as u64;
            self.download_limit.consume(chunk.len() as u64).await;
            progress(done);
        }
        file.flush().await?;
        file.sync_all().await?;
        Ok(done)
    }

    /// The thumbnail sets OneDrive has for an item (empty for most non-media files).
    pub async fn thumbnails(&self, id: &str) -> GResult<Vec<ThumbnailSet>> {
        let page: Collection<ThumbnailSet> = self
            .get_json(&self.url(&format!("/me/drive/items/{id}/thumbnails")))
            .await?;
        Ok(page.value)
    }

    /// One preview image of an item, without downloading the item itself.
    ///
    /// `size` is `small`, `medium`, `large` or a custom bounding box such as
    /// `c256x256` (the image keeps its aspect ratio and fits inside). The
    /// result is usually a JPEG. `None` when OneDrive has no thumbnail for it.
    pub async fn thumbnail(&self, id: &str, size: &str) -> GResult<Option<Bytes>> {
        const MAX: usize = 8 * 1024 * 1024;
        let url = self.url(&format!("/me/drive/items/{id}/thumbnails/0/{size}/content"));
        let resp = match self.send(|c| c.get(&url)).await {
            Ok(r) => r,
            Err(e) if e.is_not_found() => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if body.len() + chunk.len() > MAX {
                return Err(GraphError::Other("thumbnail is unexpectedly large".into()));
            }
            self.download_limit.consume(chunk.len() as u64).await;
            body.extend_from_slice(&chunk);
        }
        Ok((!body.is_empty()).then(|| Bytes::from(body)))
    }

    /// Upload a local file, as a new item or a new version of an existing one.
    ///
    /// `if_match` makes the upload fail with HTTP 412 if the item changed
    /// online since we last saw it, which is how conflicts are detected.
    pub async fn upload(
        &self,
        target: UploadTarget<'_>,
        path: &Path,
        mtime: Option<DateTime<Utc>>,
        if_match: Option<&str>,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> GResult<DriveItem> {
        let meta = tokio::fs::metadata(path).await?;
        let size = meta.len();
        let stamp = Stamp::new(&meta, mtime, if_match);
        let item = if size <= SIMPLE_UPLOAD_MAX {
            self.upload_simple(target, path, &stamp).await?
        } else {
            self.upload_session(target, path, stamp, progress).await?
        };
        // Through: a session kept from an attempt before (one with other
        // content) is no use now. What was sent stays known until the
        // caller has recorded this (see `forget_sent`): the date below is
        // another request, and an upload stopped during it has landed.
        let stale = self.sessions.lock().unwrap().remove(&target.key());
        self.discard_later(stale.into_iter().map(|s| s.upload_url).collect());
        progress(size);
        match mtime {
            Some(t) if size <= SIMPLE_UPLOAD_MAX => {
                // Keep the local modification time, as the Windows client
                // does. The answer stands for this upload only while it
                // holds what went up: a version saved over it meanwhile
                // (from another PC, say) taken for ours would never come
                // down, and the next upload from here would replace it.
                let dated = self.set_modified(&item.id, t, None).await.ok();
                Ok(dated.filter(|d| d.same_content(&item)).unwrap_or(item))
            }
            _ => Ok(item),
        }
    }

    /// Whether an upload from here sent content with this QuickXorHash to
    /// `target` since one last went through. Online, such content is this
    /// PC's own, landed without an answer, not an edit made elsewhere.
    pub fn sent_before(&self, target: UploadTarget<'_>, hash: &str) -> bool {
        self.sent
            .lock()
            .unwrap()
            .get(&target.key())
            .is_some_and(|sent| sent.iter().any(|h| h == hash))
    }

    /// An upload to `target` went through and the caller has recorded it:
    /// nothing sent to it before can land any more.
    pub fn forget_sent(&self, target: UploadTarget<'_>) {
        self.sent.lock().unwrap().remove(&target.key());
    }

    /// `hash` may be online at `key` now, sent by an upload from here.
    fn note_sent(&self, key: &str, hash: String) {
        let mut sent = self.sent.lock().unwrap();
        let hashes = sent.entry(key.to_string()).or_default();
        if !hashes.contains(&hash) {
            hashes.push(hash);
        }
    }

    /// Forget what's kept for uploads to anything but `wanted` (renamed,
    /// deleted or split off since, or the account signed out). Their
    /// sessions are cancelled online in the background.
    pub fn forget_uploads(&self, wanted: &[UploadTarget<'_>]) {
        let keep: HashSet<String> = wanted.iter().map(|t| t.key()).collect();
        self.sent.lock().unwrap().retain(|k, _| keep.contains(k));
        let mut gone = Vec::new();
        self.sessions.lock().unwrap().retain(|k, s| {
            if !keep.contains(k) {
                gone.push(s.upload_url.clone());
            }
            keep.contains(k)
        });
        self.discard_later(gone);
    }

    /// [`discard_session`](Self::discard_session) for each of these, in
    /// the background.
    fn discard_later(&self, upload_urls: Vec<String>) {
        if upload_urls.is_empty() {
            return;
        }
        let http = self.http.clone();
        tokio::spawn(async move {
            for url in upload_urls {
                let _ = http.delete(&url).send().await;
            }
        });
    }

    async fn upload_simple(
        &self,
        target: UploadTarget<'_>,
        path: &Path,
        stamp: &Stamp,
    ) -> GResult<DriveItem> {
        let mut file = tokio::fs::File::open(path).await?;
        let mut data = Vec::new();
        file.read_to_end(&mut data).await?;
        // Written to while being read, it may be half old and half new.
        if !stamp.same_file(&file.metadata().await?) {
            return Err(GraphError::Changed);
        }
        let mut hash = QuickXorHash::new();
        hash.update(&data);
        self.note_sent(&target.key(), hash.base64());
        let data = Bytes::from(data);
        self.upload_limit.consume(data.len() as u64).await;
        let url = match target {
            UploadTarget::Existing(id) => self.url(&format!("/me/drive/items/{id}/content")),
            UploadTarget::New { parent_id, name } => self.url(&format!(
                "/me/drive/items/{parent_id}:/{}:/content?@microsoft.graph.conflictBehavior=fail",
                enc(name)
            )),
        };
        let resp = self
            .send_with(
                |c| {
                    let mut rb = c
                        .put(&url)
                        .header("Content-Type", "application/octet-stream");
                    if let Some(tag) = &stamp.if_match {
                        rb = rb.header("If-Match", tag);
                    }
                    rb
                },
                Some(&data),
            )
            .await?;
        Ok(resp.json().await?)
    }

    async fn upload_session(
        &self,
        target: UploadTarget<'_>,
        path: &Path,
        stamp: Stamp,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> GResult<DriveItem> {
        let key = target.key();
        let left = self.sessions.lock().unwrap().remove(&key);
        let mut held = HeldSession {
            graph: self,
            key: &key,
            session: left,
        };
        let fresh_until = Utc::now() + chrono::Duration::minutes(5);
        let usable = held
            .session
            .as_ref()
            .map(|s| s.stamp == stamp && s.expires.is_none_or(|t| t > fresh_until));
        let mut resumed = false;
        match usable {
            // An earlier attempt at this same file was cut short: carry on,
            // unless the item was changed online since.
            Some(true) => match self.online_since(held.get(), target, path).await {
                Ok(Online::Unchanged) => resumed = true,
                Ok(Online::Uploaded(item)) => {
                    self.discard_session(&held.take().upload_url).await;
                    return Ok(*item);
                }
                Ok(Online::Changed) => self.discard_session(&held.take().upload_url).await,
                // Kept for the next attempt.
                Err(e) if e.keeps_session() => return Err(e),
                Err(e) => {
                    self.discard_session(&held.take().upload_url).await;
                    return Err(e);
                }
            },
            // The file changed since, or the session is about to run out.
            Some(false) => self.discard_session(&held.take().upload_url).await,
            None => {}
        }
        let mut restarted = false;
        loop {
            if held.session.is_none() {
                held.session = Some(self.create_upload_session(target, stamp.clone()).await?);
                resumed = false;
            }
            match self
                .upload_fragments(held.get(), resumed, target, path, &key, progress)
                .await
            {
                Ok(item) => {
                    held.take();
                    return Ok(item);
                }
                // The session is gone (it expired, say): start over, once.
                Err(e) if e.is_not_found() && !restarted => {
                    held.take();
                    restarted = true;
                }
                Err(e) if e.is_not_found() => {
                    held.take();
                    return Err(GraphError::Other(
                        "the upload session ended before the file was complete".into(),
                    ));
                }
                // Kept for the next attempt.
                Err(e) if e.keeps_session() => return Err(e),
                Err(e) => {
                    self.discard_session(&held.take().upload_url).await;
                    return Err(e);
                }
            }
        }
    }

    async fn create_upload_session(
        &self,
        target: UploadTarget<'_>,
        stamp: Stamp,
    ) -> GResult<OpenSession> {
        let mut item = json!({});
        let url = match target {
            UploadTarget::Existing(id) => {
                item["@microsoft.graph.conflictBehavior"] = json!("replace");
                self.url(&format!("/me/drive/items/{id}/createUploadSession"))
            }
            UploadTarget::New { parent_id, name } => {
                item["@microsoft.graph.conflictBehavior"] = json!("fail");
                item["name"] = json!(name);
                self.url(&format!(
                    "/me/drive/items/{parent_id}:/{}:/createUploadSession",
                    enc(name)
                ))
            }
        };
        if let Some(t) = stamp.mtime {
            item["fileSystemInfo"] = json!({ "lastModifiedDateTime": t.to_rfc3339() });
        }
        let body = json!({ "item": item });
        let resp = self
            .send(|c| {
                let mut rb = c.post(&url).json(&body);
                if let Some(tag) = &stamp.if_match {
                    rb = rb.header("If-Match", tag);
                }
                rb
            })
            .await?;
        let created: UploadSession = resp.json().await?;
        Ok(OpenSession {
            upload_url: created.upload_url,
            expires: created.expiration_date_time,
            stamp,
            sent_final: false,
            hashed: Some((QuickXorHash::new(), 0)),
        })
    }

    /// Fill an upload session, from wherever it says it got to if `resumed`.
    /// A 404 means the session is gone and the upload has to start over.
    async fn upload_fragments(
        &self,
        s: &mut OpenSession,
        resumed: bool,
        target: UploadTarget<'_>,
        path: &Path,
        key: &str,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> GResult<DriveItem> {
        let size = s.stamp.len;
        // Smaller fragments when throttled keep the rate smooth.
        let fragment = if self.upload_limit.rate() > 0 {
            FRAGMENT_UNIT
        } else {
            FRAGMENT_UNIT * 32
        };
        let mut file = tokio::fs::File::open(path).await?;
        let mut next = (!resumed).then_some(0..size);
        let mut failures = 0;
        loop {
            let (err, wait) = match next.take() {
                // After a failure only the session knows what arrived.
                None => match self.upload_status(&s.upload_url).await {
                    Ok(status) => {
                        s.expires = status.expiration_date_time.or(s.expires);
                        match status.next_range(size) {
                            Some(r) => {
                                progress(r.start);
                                next = Some(r);
                                continue;
                            }
                            None => return self.finished_upload(s, target, path).await,
                        }
                    }
                    Err((e, _)) if e.is_not_found() => {
                        return self.finished_upload(s, target, path).await
                    }
                    Err((e, wait)) if e.keeps_session() => {
                        (e, wait.unwrap_or_else(|| backoff(failures)))
                    }
                    Err((e, _)) => return Err(e),
                },
                Some(r) if r.is_empty() => return self.finished_upload(s, target, path).await,
                Some(r) => {
                    // Fragments stay multiples of 320 KiB wherever they start.
                    let (at, len) = (r.start, fragment.min(r.end - r.start));
                    let mut buf = vec![0u8; len as usize];
                    file.seek(std::io::SeekFrom::Start(at)).await?;
                    file.read_exact(&mut buf).await?;
                    // Written to since the session was made: the rest
                    // wouldn't go with what went up already.
                    if !s.stamp.same_file(&file.metadata().await?) {
                        return Err(GraphError::Changed);
                    }
                    self.upload_limit.consume(len).await;
                    s.hash(at, &buf);
                    s.sent_final |= at + len == size;
                    if let Some(hash) = s.sent_hash() {
                        self.note_sent(key, hash);
                    }

                    // The upload URL is pre-authenticated: no bearer token.
                    let range = format!("bytes {}-{}/{}", at, at + len - 1, size);
                    let rb = self
                        .upload_http
                        .put(&s.upload_url)
                        .header("Content-Range", range);
                    match send_watched(rb, Bytes::from(buf), STALL_TIMEOUT).await {
                        Ok(resp) if resp.status() == StatusCode::ACCEPTED => {
                            let status: UploadStatus = resp.json().await.unwrap_or_default();
                            s.expires = status.expiration_date_time.or(s.expires);
                            let r = status.next_range(size).unwrap_or(at + len..size);
                            progress(r.start);
                            next = Some(r);
                            failures = 0;
                            continue;
                        }
                        Ok(resp) if resp.status().is_success() => return Ok(resp.json().await?),
                        // Overlaps what the session already has.
                        Ok(resp) if resp.status() == StatusCode::RANGE_NOT_SATISFIABLE => {
                            (api_error(resp).await, Duration::ZERO)
                        }
                        Ok(resp) if retryable(resp.status()) => {
                            let wait = retry_after(&resp).unwrap_or_else(|| backoff(failures));
                            (api_error(resp).await, wait)
                        }
                        Ok(resp) => return Err(api_error(resp).await),
                        // It may or may not have arrived.
                        Err(e) if e.is_offline() => (e, backoff(failures)),
                        Err(e) => return Err(e),
                    }
                }
            };
            failures += 1;
            if failures >= MAX_ATTEMPTS {
                return Err(err);
            }
            tracing::debug!(%err, ?wait, "upload interrupted, resuming");
            tokio::time::sleep(wait).await;
        }
    }

    /// Graph checks If-Match only when a session is made, and commits a kept
    /// one over whatever is online by then, so an edit made online since has
    /// to be looked for before carrying on with it.
    async fn online_since(
        &self,
        s: &OpenSession,
        target: UploadTarget<'_>,
        path: &Path,
    ) -> GResult<Online> {
        let (UploadTarget::Existing(id), Some(tag)) = (target, &s.stamp.if_match) else {
            return Ok(Online::Unchanged);
        };
        let item = match self.item(id).await {
            Ok(item) => item,
            Err(e) if e.is_not_found() => return Ok(Online::Changed),
            Err(e) => return Err(e),
        };
        if item.e_tag.as_ref() == Some(tag) {
            return Ok(Online::Unchanged);
        }
        if s.sent_final {
            if let Some(item) = self.same_content(Some(item), s, path).await? {
                return Ok(Online::Uploaded(Box::new(item)));
            }
        }
        Ok(Online::Changed)
    }

    /// What an upload session has received so far. A refusal comes with
    /// how long Graph asked to wait, if it did.
    async fn upload_status(
        &self,
        upload_url: &str,
    ) -> Result<UploadStatus, (GraphError, Option<Duration>)> {
        let resp = self
            .http
            .get(upload_url)
            .send()
            .await
            .map_err(|e| (e.into(), None))?;
        if !resp.status().is_success() {
            let wait = retry_after(&resp);
            return Err((api_error(resp).await, wait));
        }
        resp.json().await.map_err(|e| (e.into(), None))
    }

    /// Best effort: free a server-side session that won't be used again.
    async fn discard_session(&self, upload_url: &str) {
        let _ = self.http.delete(upload_url).send().await;
    }

    /// An upload session vanished, or has every byte, with no reply naming
    /// the item. If the last fragment went up, that reply was probably lost
    /// and the file is online; it counts if its content matches. Otherwise a
    /// 404 says to start over.
    async fn finished_upload(
        &self,
        s: &OpenSession,
        target: UploadTarget<'_>,
        path: &Path,
    ) -> GResult<DriveItem> {
        if s.sent_final {
            let online = match target {
                UploadTarget::Existing(id) => match self.item(id).await {
                    Ok(item) => Some(item),
                    Err(e) if e.is_not_found() => None,
                    Err(e) => return Err(e),
                },
                UploadTarget::New { parent_id, name } => self.child(parent_id, name).await?,
            };
            if let Some(item) = self.same_content(online, s, path).await? {
                return Ok(item);
            }
        }
        Err(GraphError::Api {
            status: 404,
            code: "itemNotFound".into(),
            message: "the upload session is gone".into(),
        })
    }

    /// `online` if it has the content of the file the session is for.
    async fn same_content(
        &self,
        online: Option<DriveItem>,
        s: &OpenSession,
        path: &Path,
    ) -> GResult<Option<DriveItem>> {
        let Some(item) = online.filter(|i| i.size == Some(s.stamp.len)) else {
            return Ok(None);
        };
        let local = path.to_path_buf();
        let hash = tokio::task::spawn_blocking(move || QuickXorHash::of_file(&local))
            .await
            .map_err(|e| GraphError::Other(e.to_string()))??;
        Ok((item.quick_xor_hash() == Some(hash.as_str())).then_some(item))
    }

    /// Create (or fetch the existing) sharing link. `kind` is "view" or "edit";
    /// `scope` is "anonymous" or "organization".
    pub async fn create_link(&self, id: &str, kind: &str, scope: &str) -> GResult<String> {
        let url = self.url(&format!("/me/drive/items/{id}/createLink"));
        let p: Permission = self
            .send_json(Method::POST, &url, &json!({ "type": kind, "scope": scope }))
            .await?;
        p.link
            .and_then(|l| l.web_url)
            .ok_or_else(|| GraphError::Other("no link in createLink response".into()))
    }

    pub async fn versions(&self, id: &str) -> GResult<Vec<DriveItemVersion>> {
        let page: Collection<DriveItemVersion> = self
            .get_json(&self.url(&format!("/me/drive/items/{id}/versions")))
            .await?;
        Ok(page.value)
    }

    pub async fn restore_version(&self, id: &str, version_id: &str) -> GResult<()> {
        let url = self.url(&format!(
            "/me/drive/items/{id}/versions/{version_id}/restoreVersion"
        ));
        self.send(|c| c.post(&url).header("Content-Length", 0))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpSocket;

    const MIB: usize = 1024 * 1024;

    /// A one-request HTTP server that reads the body with pauses (at byte
    /// offset, for how long) and then answers 201.
    async fn slow_server(pauses: Vec<(usize, Duration)>) -> String {
        let socket = TcpSocket::new_v4().unwrap();
        // A small window, so the client can't park the whole body in the kernel.
        socket.set_recv_buffer_size(64 * 1024).unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let listener = socket.listen(1).unwrap();
        let url = format!("http://{}/upload", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(conn.read_u8().await.unwrap());
            }
            let head = String::from_utf8(head).unwrap().to_ascii_lowercase();
            let len: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let mut pauses = pauses.into_iter().peekable();
            let mut buf = vec![0u8; 64 * 1024];
            let mut done = 0;
            while done < len {
                if let Some((_, pause)) = pauses.next_if(|(at, _)| done >= *at) {
                    tokio::time::sleep(pause).await;
                }
                done += conn.read(&mut buf).await.unwrap();
            }
            conn.write_all(b"HTTP/1.1 201 Created\r\ncontent-length: 2\r\n\r\n{}")
                .await
                .unwrap();
        });
        url
    }

    #[tokio::test]
    async fn an_upload_may_take_longer_than_the_timeout_while_it_keeps_moving() {
        let stall = Duration::from_secs(1);
        let pauses = (1..=5)
            .map(|i| (i * 4 * MIB, Duration::from_millis(400)))
            .collect();
        let url = slow_server(pauses).await;
        let started = std::time::Instant::now();
        let body = Bytes::from(vec![7u8; 64 * MIB]);
        let resp = send_watched(reqwest::Client::new().put(&url), body, stall)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        assert!(started.elapsed() > stall, "{:?}", started.elapsed());
    }

    #[tokio::test]
    async fn an_upload_that_stops_moving_fails_as_offline() {
        let url = slow_server(vec![(0, Duration::from_secs(60))]).await;
        let body = Bytes::from(vec![7u8; 64 * MIB]);
        let err = send_watched(
            reqwest::Client::new().put(&url),
            body,
            Duration::from_millis(300),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GraphError::Stalled), "{err}");
        assert!(err.is_offline());
    }

    #[test]
    fn next_expected_ranges() {
        let status = |ranges: &[&str]| UploadStatus {
            next_expected_ranges: ranges.iter().map(|r| r.to_string()).collect(),
            ..Default::default()
        };
        assert_eq!(status(&["26-"]).next_range(100), Some(26..100));
        assert_eq!(status(&["12-55", "77-99"]).next_range(100), Some(12..56));
        assert_eq!(status(&["0-"]).next_range(0), None);
        assert_eq!(status(&[]).next_range(100), None);
        assert_eq!(status(&["junk"]).next_range(100), None);
    }
}
