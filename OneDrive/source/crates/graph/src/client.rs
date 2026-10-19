//! HTTP client for the OneDrive endpoints of Microsoft Graph.

use std::path::Path;
use std::sync::{Arc, RwLock};
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
use crate::throttle::RateLimiter;
use crate::types::*;

/// Files up to this size go up in a single PUT; larger ones use an upload session.
pub const SIMPLE_UPLOAD_MAX: u64 = 4 * 1024 * 1024;
/// Upload-session fragments must be multiples of 320 KiB.
const FRAGMENT_UNIT: u64 = 320 * 1024;
const MAX_ATTEMPTS: u32 = 6;

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
        matches!(self, GraphError::Network(e) if e.is_connect() || e.is_timeout() || e.is_request())
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

pub struct Graph {
    http: reqwest::Client,
    base: RwLock<String>,
    tokens: Arc<TokenStore>,
    pub upload_limit: RateLimiter,
    pub download_limit: RateLimiter,
}

impl Graph {
    pub fn new(base: &str, tokens: Arc<TokenStore>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!(
                "AxentraOneDrive/",
                env!("CARGO_PKG_VERSION"),
                " (Linux)"
            ))
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(120))
            .build()
            .expect("building HTTP client");
        Self {
            http,
            base: RwLock::new(base.trim_end_matches('/').to_string()),
            tokens,
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
        let mut attempt = 0;
        let mut refreshed = false;
        loop {
            let token = self.tokens.access_token().await?;
            match build(&self.http).bearer_auth(&token).send().await {
                Err(e) if attempt + 1 < MAX_ATTEMPTS && (e.is_timeout() || e.is_connect()) => {
                    tokio::time::sleep(backoff(attempt)).await;
                    attempt += 1;
                }
                Err(e) => return Err(e.into()),
                Ok(resp) if resp.status() == StatusCode::UNAUTHORIZED && !refreshed => {
                    self.tokens.invalidate().await;
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

    pub async fn set_modified(&self, id: &str, mtime: DateTime<Utc>) -> GResult<DriveItem> {
        let url = self.url(&format!("/me/drive/items/{id}"));
        let body = json!({ "fileSystemInfo": { "lastModifiedDateTime": mtime.to_rfc3339() } });
        self.send_json(Method::PATCH, &url, &body).await
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
        let size = tokio::fs::metadata(path).await?.len();
        let item = if size <= SIMPLE_UPLOAD_MAX {
            self.upload_simple(target, path, if_match).await?
        } else {
            self.upload_session(target, path, size, mtime, if_match, progress)
                .await?
        };
        progress(size);
        match mtime {
            Some(t) if size <= SIMPLE_UPLOAD_MAX => {
                // Keep the local modification time, as the Windows client does.
                Ok(self.set_modified(&item.id, t).await.unwrap_or(item))
            }
            _ => Ok(item),
        }
    }

    async fn upload_simple(
        &self,
        target: UploadTarget<'_>,
        path: &Path,
        if_match: Option<&str>,
    ) -> GResult<DriveItem> {
        let data = Bytes::from(tokio::fs::read(path).await?);
        self.upload_limit.consume(data.len() as u64).await;
        let url = match target {
            UploadTarget::Existing(id) => self.url(&format!("/me/drive/items/{id}/content")),
            UploadTarget::New { parent_id, name } => self.url(&format!(
                "/me/drive/items/{parent_id}:/{}:/content?@microsoft.graph.conflictBehavior=fail",
                enc(name)
            )),
        };
        let resp = self
            .send(|c| {
                let mut rb = c
                    .put(&url)
                    .header("Content-Type", "application/octet-stream")
                    .body(data.clone());
                if let Some(tag) = if_match {
                    rb = rb.header("If-Match", tag);
                }
                rb
            })
            .await?;
        Ok(resp.json().await?)
    }

    async fn upload_session(
        &self,
        target: UploadTarget<'_>,
        path: &Path,
        size: u64,
        mtime: Option<DateTime<Utc>>,
        if_match: Option<&str>,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> GResult<DriveItem> {
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
        if let Some(t) = mtime {
            item["fileSystemInfo"] = json!({ "lastModifiedDateTime": t.to_rfc3339() });
        }
        let body = json!({ "item": item });
        let session: UploadSession = {
            let resp = self
                .send(|c| {
                    let mut rb = c.post(&url).json(&body);
                    if let Some(tag) = if_match {
                        rb = rb.header("If-Match", tag);
                    }
                    rb
                })
                .await?;
            resp.json().await?
        };

        let result = self
            .upload_fragments(&session.upload_url, path, size, progress)
            .await;
        if result.is_err() {
            // Best effort: free the server-side session.
            let _ = self.http.delete(&session.upload_url).send().await;
        }
        result
    }

    async fn upload_fragments(
        &self,
        upload_url: &str,
        path: &Path,
        size: u64,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> GResult<DriveItem> {
        // Smaller fragments when throttled keep the rate smooth.
        let fragment = if self.upload_limit.rate() > 0 {
            FRAGMENT_UNIT
        } else {
            FRAGMENT_UNIT * 32
        };
        let mut file = tokio::fs::File::open(path).await?;
        let mut offset = 0u64;
        while offset < size {
            let len = fragment.min(size - offset);
            let mut buf = vec![0u8; len as usize];
            file.seek(std::io::SeekFrom::Start(offset)).await?;
            file.read_exact(&mut buf).await?;
            let chunk = Bytes::from(buf);
            self.upload_limit.consume(len).await;

            let range = format!("bytes {}-{}/{}", offset, offset + len - 1, size);
            let mut attempt = 0;
            let resp = loop {
                // The upload URL is pre-authenticated: no bearer token.
                let sent = self
                    .http
                    .put(upload_url)
                    .header("Content-Length", len)
                    .header("Content-Range", &range)
                    .body(chunk.clone())
                    .send()
                    .await;
                match sent {
                    Ok(r) if retryable(r.status()) && attempt + 1 < MAX_ATTEMPTS => {
                        tokio::time::sleep(retry_after(&r).unwrap_or_else(|| backoff(attempt)))
                            .await;
                    }
                    Err(e) if attempt + 1 < MAX_ATTEMPTS && (e.is_timeout() || e.is_connect()) => {
                        tokio::time::sleep(backoff(attempt)).await;
                    }
                    Ok(r) if r.status().is_success() => break r,
                    Ok(r) => return Err(api_error(r).await),
                    Err(e) => return Err(e.into()),
                }
                attempt += 1;
            };
            offset += len;
            progress(offset);
            if matches!(resp.status(), StatusCode::OK | StatusCode::CREATED) {
                return Ok(resp.json().await?);
            }
        }
        Err(GraphError::Other(
            "upload session finished without returning the item".into(),
        ))
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
