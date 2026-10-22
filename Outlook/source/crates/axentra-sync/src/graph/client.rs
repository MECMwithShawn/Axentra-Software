//! Microsoft Graph REST client (§4.1–4.3): immutable ids and UTC on every
//! request, opaque paging links, Retry-After / exponential backoff on
//! 429 / 503 / 504, one token refresh on 401, and at most three concurrent
//! requests per mailbox.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use reqwest::{Method, StatusCode};
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::error::{SyncError, SyncResult, copy, from_reqwest};
use crate::secrets::Secret;

pub const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
/// Concurrent requests per mailbox (one below Outlook's limit of 4).
const CONCURRENCY: usize = 3;
const MAX_ATTEMPTS: u32 = 6;

/// Supplies access tokens; `force` asks for a fresh one after a 401.
pub trait TokenSource: Send + Sync {
    fn token(&self, force: bool) -> BoxFuture<'_, SyncResult<Secret>>;
}

/// A fixed token (tests, and the first calls right after sign-in).
pub struct StaticToken(pub Secret);

impl TokenSource for StaticToken {
    fn token(&self, _force: bool) -> BoxFuture<'_, SyncResult<Secret>> {
        let t = self.0.clone();
        Box::pin(async move { Ok(t) })
    }
}

#[derive(Clone)]
pub struct Graph {
    http: reqwest::Client,
    base: String,
    tokens: Arc<dyn TokenSource>,
    limit: Arc<Semaphore>,
}

/// `Retry-After` as seconds or an HTTP-date.
pub fn parse_retry_after(value: &str, now: DateTime<Utc>) -> Option<Duration> {
    let v = value.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let when = DateTime::parse_from_rfc2822(v).ok()?.with_timezone(&Utc);
    Some((when - now).to_std().unwrap_or(Duration::ZERO))
}

/// 2 s, 4 s, 8 s … capped at 5 minutes, ±20 % jitter.
pub fn backoff(attempt: u32) -> Duration {
    let base = 2u64.saturating_pow(attempt.clamp(1, 9)).min(300) as f64;
    let mut b = [0u8; 2];
    let _ = getrandom::fill(&mut b);
    let jitter = (u16::from_le_bytes(b) as f64 / u16::MAX as f64) * 0.4 - 0.2;
    Duration::from_secs_f64((base * (1.0 + jitter)).max(1.0))
}

/// Map a Graph error response (§4.1 table).
pub fn graph_error(status: u16, body: &[u8]) -> SyncError {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let code = v["error"]["code"].as_str().unwrap_or_default();
    let message = v["error"]["message"].as_str().unwrap_or_default();
    match (status, code) {
        (_, "MailboxNotEnabledForRESTAPI") | (_, "MailboxNotSupportedForRESTAPI") => {
            SyncError::User("We couldn't find a mailbox for this account in Microsoft 365.".into())
        }
        (_, "ErrorQuotaExceeded") => SyncError::User(copy::MAILBOX_FULL.into()),
        (410, _) | (_, "syncStateNotFound") | (_, "SyncStateNotFound") | (_, "resyncRequired") | (_, "SyncStateInvalid") => {
            SyncError::ResyncRequired
        }
        (404, _) | (_, "ErrorItemNotFound") => SyncError::NotFound,
        (401, _) => SyncError::Auth("Your sign-in has expired. Sign in again.".into()),
        (403, "ErrorAccessDenied") | (403, "Authorization_RequestDenied") => {
            SyncError::User(format!("Access was denied by Microsoft 365. {message}").trim().to_string())
        }
        _ => SyncError::protocol(format!("Microsoft 365 returned an error ({status} {code}). {message}").trim().to_string()),
    }
}

/// A response body of at most `max` bytes, or `None` as soon as it is
/// longer (or says it will be). Bodies arrive decompressed, so the limit is
/// on what would be kept, whatever the transfer encoding.
async fn read_at_most(mut resp: reqwest::Response, max: usize) -> SyncResult<Option<Vec<u8>>> {
    if resp.content_length().is_some_and(|n| n > max as u64) {
        return Ok(None);
    }
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| from_reqwest(&e))? {
        if out.len().saturating_add(chunk.len()) > max {
            return Ok(None);
        }
        out.extend_from_slice(&chunk);
    }
    Ok(Some(out))
}

impl Graph {
    pub fn new(http: reqwest::Client, tokens: Arc<dyn TokenSource>) -> Self {
        Self::with_base(http, tokens, GRAPH_BASE)
    }

    pub fn with_base(http: reqwest::Client, tokens: Arc<dyn TokenSource>, base: &str) -> Self {
        Graph { http, base: base.trim_end_matches('/').to_string(), tokens, limit: Arc::new(Semaphore::new(CONCURRENCY)) }
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}{}", self.base, path)
        }
    }

    /// One request with retries. Returns status and body.
    pub async fn request(&self, method: Method, path: &str, body: Option<RequestBody>, prefer: &[&str]) -> SyncResult<(StatusCode, Vec<u8>)> {
        let (status, bytes) = self.request_status(method, path, body, prefer).await?;
        if status.is_success() { Ok((status, bytes)) } else { Err(graph_error(status.as_u16(), &bytes)) }
    }

    /// [`Graph::request`] (same retries, throttling and token refresh) that
    /// returns a final error status with its body instead of mapping it to
    /// a [`SyncError`], for callers that tell one status from another.
    pub async fn request_status(&self, method: Method, path: &str, body: Option<RequestBody>, prefer: &[&str]) -> SyncResult<(StatusCode, Vec<u8>)> {
        let (status, bytes) = self.exchange(method, path, body, prefer, None).await?;
        Ok((status, bytes.unwrap_or_default()))
    }

    /// [`Graph::request_status`] that reads at most `max` body bytes when
    /// given: the body is `None` when it is longer (the rest isn't read, and
    /// a declared `Content-Length` over the limit isn't read at all).
    async fn exchange(
        &self,
        method: Method,
        path: &str,
        body: Option<RequestBody>,
        prefer: &[&str],
        max: Option<usize>,
    ) -> SyncResult<(StatusCode, Option<Vec<u8>>)> {
        let url = self.url(path);
        let mut forced = false;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let permit = self.limit.acquire().await.map_err(|_| SyncError::Cancelled)?;
            let token = self.tokens.token(forced).await?;
            let mut req = self
                .http
                .request(method.clone(), &url)
                .bearer_auth(token.expose())
                .header("Prefer", "IdType=\"ImmutableId\"")
                .header("Prefer", "outlook.timezone=\"UTC\"")
                .header("Prefer", "outlook.body-content-type=\"html\"")
                .header("client-request-id", uuid::Uuid::new_v4().to_string())
                .timeout(Duration::from_secs(120));
            for p in prefer {
                req = req.header("Prefer", *p);
            }
            req = match &body {
                Some(RequestBody::Json(v)) => req.json(v),
                Some(RequestBody::Text(content_type, text)) => req.header("Content-Type", *content_type).body(text.clone()),
                None if method == Method::POST => req.header("Content-Length", "0"),
                None => req,
            };
            let resp = req.send().await.map_err(|e| from_reqwest(&e))?;
            let status = resp.status();
            let retry_after = resp.headers().get("Retry-After").and_then(|v| v.to_str().ok()).and_then(|v| parse_retry_after(v, Utc::now()));
            let bytes = match max {
                None => Some(resp.bytes().await.map_err(|e| from_reqwest(&e))?.to_vec()),
                Some(max) => read_at_most(resp, max).await?,
            };
            drop(permit);
            if status.is_success() {
                return Ok((status, bytes));
            }
            match status.as_u16() {
                401 if !forced => {
                    forced = true;
                    continue;
                }
                429 | 503 | 504 | 502 => {
                    if attempt >= MAX_ATTEMPTS {
                        return Err(SyncError::Throttled { retry_after });
                    }
                    let wait = retry_after.unwrap_or_else(|| backoff(attempt));
                    tracing::debug!("Graph {} {status}; retrying in {:?}", method, wait);
                    tokio::time::sleep(wait).await;
                }
                _ => return Ok((status, bytes)),
            }
        }
    }

    fn json(bytes: &[u8]) -> SyncResult<Value> {
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(bytes).map_err(|_| SyncError::protocol("Microsoft 365 sent an unexpected response."))
    }

    pub async fn get(&self, path: &str) -> SyncResult<Value> {
        let (_, b) = self.request(Method::GET, path, None, &[]).await?;
        Self::json(&b)
    }

    pub async fn get_bytes(&self, path: &str) -> SyncResult<Vec<u8>> {
        Ok(self.request(Method::GET, path, None, &[]).await?.1)
    }

    /// GET returning the final HTTP status and body, errors included
    /// (transport failures and exhausted retries are still `Err`).
    pub async fn get_status(&self, path: &str) -> SyncResult<(u16, Vec<u8>)> {
        let (status, body) = self.request_status(Method::GET, path, None, &[]).await?;
        Ok((status.as_u16(), body))
    }

    /// [`Graph::get_status`] for a body of at most `max` bytes: `None` for a
    /// longer one, which is not downloaded past the limit.
    pub async fn get_status_max(&self, path: &str, max: usize) -> SyncResult<(u16, Option<Vec<u8>>)> {
        let (status, body) = self.exchange(Method::GET, path, None, &[], Some(max)).await?;
        Ok((status.as_u16(), body))
    }

    /// Every item of a collection, following `@odata.nextLink` verbatim.
    pub async fn get_all(&self, path: &str) -> SyncResult<Vec<Value>> {
        let mut out = Vec::new();
        let mut next = Some(path.to_string());
        while let Some(url) = next.take() {
            let (_, b) = self.request(Method::GET, &url, None, &["odata.maxpagesize=100"]).await?;
            let mut page = Self::json(&b)?;
            if let Some(items) = page["value"].as_array_mut() {
                out.append(items);
            }
            next = page["@odata.nextLink"].as_str().map(str::to_string);
        }
        Ok(out)
    }

    /// One delta round: pages until a `@odata.deltaLink`.
    pub async fn delta(&self, url: &str) -> SyncResult<(Vec<Value>, Option<String>)> {
        let mut out = Vec::new();
        let mut next = Some(url.to_string());
        while let Some(u) = next.take() {
            let (_, b) = self.request(Method::GET, &u, None, &["odata.maxpagesize=100"]).await?;
            let mut page = Self::json(&b)?;
            if let Some(items) = page["value"].as_array_mut() {
                out.append(items);
            }
            if let Some(delta) = page["@odata.deltaLink"].as_str() {
                return Ok((out, Some(delta.to_string())));
            }
            next = page["@odata.nextLink"].as_str().map(str::to_string);
        }
        Ok((out, None))
    }

    pub async fn post(&self, path: &str, body: Value) -> SyncResult<Value> {
        let (_, b) = self.request(Method::POST, path, Some(RequestBody::Json(body)), &[]).await?;
        Self::json(&b)
    }

    /// POST without a body (e.g. `/send`).
    pub async fn post_empty(&self, path: &str) -> SyncResult<()> {
        self.request(Method::POST, path, None, &[]).await.map(|_| ())
    }

    pub async fn post_text(&self, path: &str, content_type: &'static str, text: String) -> SyncResult<()> {
        self.request(Method::POST, path, Some(RequestBody::Text(content_type, text)), &[]).await.map(|_| ())
    }

    pub async fn patch(&self, path: &str, body: Value) -> SyncResult<Value> {
        let (_, b) = self.request(Method::PATCH, path, Some(RequestBody::Json(body)), &[]).await?;
        Self::json(&b)
    }

    pub async fn delete(&self, path: &str) -> SyncResult<()> {
        match self.request(Method::DELETE, path, None, &[]).await {
            Ok(_) | Err(SyncError::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

pub enum RequestBody {
    Json(Value),
    Text(&'static str, String),
}

/// Percent-encode one path segment (Graph ids may contain `/` and `+`).
pub fn seg(id: &str) -> String {
    url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>().replace('+', "%20")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn retry_after_parsing_and_backoff() {
        let now = DateTime::parse_from_rfc3339("2026-10-19T12:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(parse_retry_after("120", now), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after("Mon, 19 Oct 2026 12:00:30 GMT", now), Some(Duration::from_secs(30)));
        assert_eq!(parse_retry_after("Mon, 19 Oct 2026 11:00:00 GMT", now), Some(Duration::ZERO));
        assert_eq!(parse_retry_after("soon", now), None);
        for attempt in 1..12 {
            let b = backoff(attempt).as_secs_f64();
            let base = 2f64.powi(attempt.min(9) as i32).min(300.0);
            assert!(b >= base * 0.79 && b <= base * 1.21, "attempt {attempt}: {b}");
        }
    }

    #[test]
    fn error_envelope_mapping() {
        let body = |code: &str| format!(r#"{{"error":{{"code":"{code}","message":"m"}}}}"#).into_bytes();
        assert!(matches!(graph_error(404, &body("ErrorItemNotFound")), SyncError::NotFound));
        assert!(matches!(graph_error(410, b""), SyncError::ResyncRequired));
        assert!(matches!(graph_error(400, &body("syncStateNotFound")), SyncError::ResyncRequired));
        assert_eq!(graph_error(403, &body("ErrorQuotaExceeded")).to_string(), copy::MAILBOX_FULL);
        assert!(graph_error(400, &body("MailboxNotEnabledForRESTAPI")).to_string().contains("couldn't find a mailbox"));
        assert_eq!(seg("AAMk/a+b="), "AAMk%2Fa%2Bb%3D");
    }

    /// Serves a 429 with Retry-After first, then two pages linked by nextLink.
    #[tokio::test]
    async fn paging_follows_next_link_and_honours_retry_after() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let seen_prefer = Arc::new(std::sync::Mutex::new(String::new()));
        let (h, p, b) = (hits.clone(), seen_prefer.clone(), base.clone());
        tokio::spawn(async move {
            loop {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                *p.lock().unwrap() = req.clone();
                let n = h.fetch_add(1, Ordering::SeqCst);
                let (status, extra, body) = if n == 0 {
                    ("429 Too Many Requests", "Retry-After: 1\r\n", String::new())
                } else if req.starts_with("GET /me/messages?page=2") {
                    ("200 OK", "", r#"{"value":[{"id":"3"}]}"#.to_string())
                } else {
                    ("200 OK", "", format!(r#"{{"value":[{{"id":"1"}},{{"id":"2"}}],"@odata.nextLink":"{b}/me/messages?page=2"}}"#))
                };
                let resp = format!("HTTP/1.1 {status}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                s.write_all(resp.as_bytes()).await.unwrap();
            }
        });
        let graph = Graph::with_base(reqwest::Client::new(), Arc::new(StaticToken(Secret::new("tok"))), &base);
        let started = std::time::Instant::now();
        let items = graph.get_all("/me/messages").await.unwrap();
        assert_eq!(items.iter().map(|v| v["id"].as_str().unwrap()).collect::<Vec<_>>(), vec!["1", "2", "3"]);
        assert!(started.elapsed() >= Duration::from_millis(900), "waited for Retry-After");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        let last = seen_prefer.lock().unwrap().to_ascii_lowercase();
        assert!(last.contains("prefer: idtype=\"immutableid\""));
        assert!(last.contains("authorization: bearer tok"));
    }
}
