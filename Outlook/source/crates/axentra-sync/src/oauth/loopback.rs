//! Authorization code + PKCE with a loopback redirect (§3.4): listen on a
//! random loopback port (IPv4, and IPv6 when available), redirect to
//! `http://localhost:{port}`, send the browser to the authorize URL, accept
//! exactly one callback whose `state` matches, answer with a small page and
//! redeem the code at once.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{OAuthClient, Pkce, TokenSet, authorize_url, pkce, random_token, redeem_code};
use crate::error::{SyncError, SyncResult, copy};

/// How long the listener waits for the browser (estimate, §3.4).
pub const BROWSER_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// What arrived on the redirect URI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Callback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Parse a raw HTTP request (GET with a query, or POST form) aimed at `/`.
/// Returns `None` for other paths (favicon etc.).
pub fn parse_callback(request: &str) -> Option<Callback> {
    let mut lines = request.split("\r\n");
    let first = lines.next()?;
    let mut parts = first.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/" {
        return None;
    }
    let params = match method {
        "GET" => query.to_string(),
        "POST" => request.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default(),
        _ => return None,
    };
    let mut cb = Callback::default();
    for (k, v) in url::form_urlencoded::parse(params.as_bytes()) {
        match k.as_ref() {
            "code" => cb.code = Some(v.into_owned()),
            "state" => cb.state = Some(v.into_owned()),
            "error" => cb.error = Some(v.into_owned()),
            "error_description" => cb.error_description = Some(v.into_owned()),
            _ => {}
        }
    }
    Some(cb)
}

fn page(title: &str, text: &str) -> String {
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{title}</title>\
         <style>body{{font:16px system-ui,sans-serif;margin:15vh auto;max-width:480px;padding:0 16px;color:#242424}}\
         @media (prefers-color-scheme:dark){{body{{background:#292929;color:#fff}}}}</style></head>\
         <body><h1 style=\"font-size:20px\">{title}</h1><p>{text}</p></body></html>"
    );
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Map an `error` from the callback to user-facing copy (§2.4).
pub fn callback_error(error: &str, description: &str) -> SyncError {
    let has = |code: &str| description.contains(code);
    if has("AADSTS53003") {
        return SyncError::User(copy::CA_BLOCKED.into());
    }
    if has("AADSTS65001") || has("AADSTS90094") || has("AADSTS90095") || error == "consent_required" {
        return SyncError::User(copy::ADMIN_CONSENT.into());
    }
    match error {
        "access_denied" => SyncError::User(copy::CANCELLED.into()),
        "interaction_required" | "login_required" => SyncError::User("Sign-in needs more steps. Please try again.".into()),
        _ => SyncError::User(format!("Sign-in failed ({error}). Please try again.")),
    }
}

async fn read_request(sock: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(10), sock.read(&mut chunk)).await.ok()?.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 64 * 1024 {
            return None;
        }
        let text = String::from_utf8_lossy(&buf);
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let len = head
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            if body.len() >= len {
                return Some(text.into_owned());
            }
        }
    }
    (!buf.is_empty()).then(|| String::from_utf8_lossy(&buf).into_owned())
}

/// Accept on the IPv4 listener, or on the IPv6 one when the browser resolved
/// `localhost` to `::1`.
async fn accept_any(v4: &TcpListener, v6: Option<&TcpListener>) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    match v6 {
        Some(l6) => tokio::select! { r = v4.accept() => r, r = l6.accept() => r },
        None => v4.accept().await,
    }
}

/// A sign-in waiting for the browser.
pub struct BrowserSignIn {
    /// Open this in the browser (also offered as "Copy sign-in link").
    pub auth_url: String,
    pub redirect_uri: String,
    listener: TcpListener,
    /// `[::1]` on the same port, for browsers that resolve localhost to IPv6 first.
    listener6: Option<TcpListener>,
    state: String,
    pkce: Pkce,
    client: OAuthClient,
    scopes: String,
}

impl BrowserSignIn {
    /// Bind the listener and build the authorize URL.
    pub async fn begin(client: OAuthClient, scopes: &str, login_hint: Option<&str>, select_account: bool) -> SyncResult<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| SyncError::protocol(format!("Couldn't start the sign-in listener: {e}")))?;
        let port = listener.local_addr().map_err(|e| SyncError::protocol(e.to_string()))?.port();
        let listener6 = TcpListener::bind(("::1", port)).await.ok();
        // No path and no trailing slash. Microsoft matches a registered
        // `http://localhost` against any port; `127.0.0.1` needs its own entry,
        // and personal-account sign-in (login.live.com) rejects it unregistered.
        let redirect_uri = format!("http://localhost:{port}");
        let state = random_token();
        let pkce = pkce();
        let auth_url = authorize_url(&client, scopes, &redirect_uri, &state, &pkce.challenge, login_hint, select_account);
        Ok(BrowserSignIn { auth_url, redirect_uri, listener, listener6, state, pkce, client, scopes: scopes.to_string() })
    }

    /// Wait for the callback (up to 10 minutes) and redeem the code.
    /// Dropping the future closes the listener and discards the verifier.
    pub async fn finish(self, http: &reqwest::Client) -> SyncResult<TokenSet> {
        let callback = tokio::time::timeout(BROWSER_TIMEOUT, async {
            // Each connection is read on its own task: browsers open idle
            // speculative connections that must not hold up the real callback.
            let (tx, mut rx) = tokio::sync::mpsc::channel::<(TcpStream, Callback)>(4);
            loop {
                let (mut sock, cb) = tokio::select! {
                    accepted = accept_any(&self.listener, self.listener6.as_ref()) => {
                        let (mut sock, _) = accepted.map_err(|e| SyncError::protocol(e.to_string()))?;
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let Some(request) = read_request(&mut sock).await else { return };
                            match parse_callback(&request) {
                                Some(cb) => {
                                    let _ = tx.send((sock, cb)).await;
                                }
                                None => {
                                    let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                                }
                            }
                        });
                        continue;
                    }
                    Some(got) = rx.recv() => got,
                };
                if cb.state.as_deref() != Some(self.state.as_str()) {
                    let _ = sock.write_all(page("Sign-in link expired", "Start signing in again from Axentra Outlook.").as_bytes()).await;
                    continue;
                }
                let reply = if cb.error.is_some() {
                    page("Sign-in didn't finish", "Return to Axentra Outlook to try again.")
                } else {
                    page("You're signed in", "You're signed in. You can close this tab and return to Axentra Outlook.")
                };
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
                return Ok::<Callback, SyncError>(cb);
            }
        })
        .await
        .map_err(|_| SyncError::User(copy::SIGNIN_TIMEOUT.into()))??;
        if let Some(error) = &callback.error {
            return Err(callback_error(error, callback.error_description.as_deref().unwrap_or_default()));
        }
        let code = callback.code.ok_or_else(|| SyncError::protocol("The sign-in response had no code."))?;
        redeem_code(http, &self.client, &code, &self.pkce.verifier, &self.redirect_uri, &self.scopes).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_parsing() {
        let get = "GET /?code=0.AXo%2Bab&state=xyz HTTP/1.1\r\nHost: 127.0.0.1:5000\r\n\r\n";
        assert_eq!(parse_callback(get), Some(Callback { code: Some("0.AXo+ab".into()), state: Some("xyz".into()), ..Default::default() }));
        let post = "POST / HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 47\r\n\r\nerror=access_denied&error_description=AADSTS65004&state=s";
        let cb = parse_callback(post).unwrap();
        assert_eq!(cb.error.as_deref(), Some("access_denied"));
        assert_eq!(parse_callback("GET /favicon.ico HTTP/1.1\r\n\r\n"), None);
        assert_eq!(callback_error("access_denied", "AADSTS65004: User declined").to_string(), copy::CANCELLED);
        assert_eq!(callback_error("invalid_request", "AADSTS90094: admin").to_string(), copy::ADMIN_CONSENT);
    }

    #[tokio::test]
    async fn listener_accepts_only_the_matching_state() {
        let client = OAuthClient::microsoft("common", "cid");
        let flow = BrowserSignIn::begin(client, super::super::GRAPH_SCOPES, Some("a@b.c"), false).await.unwrap();
        assert!(flow.auth_url.contains(&format!("state={}", flow.state)));
        let port: u16 = flow.redirect_uri.rsplit(':').next().unwrap().parse().unwrap();
        let state = flow.state.clone();
        let http = reqwest::Client::new();
        let waiter = tokio::spawn(async move { flow.finish(&http).await });
        let send = |req: String| async move {
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(req.as_bytes()).await.unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out).await;
            out
        };
        // An idle (speculative) connection doesn't hold up the others.
        let _idle = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let started = std::time::Instant::now();
        assert!(send("GET /favicon.ico HTTP/1.1\r\n\r\n".into()).await.starts_with("HTTP/1.1 404"));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(send("GET /?code=x&state=wrong HTTP/1.1\r\n\r\n".into()).await.contains("expired"));
        let page = send(format!("GET /?error=access_denied&state={state} HTTP/1.1\r\n\r\n")).await;
        assert!(page.contains("didn't finish"));
        let err = waiter.await.unwrap().unwrap_err();
        assert_eq!(err.to_string(), copy::CANCELLED);
    }
}
