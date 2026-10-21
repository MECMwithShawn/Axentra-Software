//! Microsoft identity platform sign-in for a public (desktop) client.
//!
//! Two flows, both without a client secret:
//! * authorization code + PKCE with a loopback redirect (opens the browser,
//!   like the Windows client's sign-in window), and
//! * device code, for headless machines.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::client::GraphError;

pub const SCOPES: &str = "Files.ReadWrite.All User.Read offline_access";

/// Refresh this long before the access token actually expires.
const EXPIRY_MARGIN_SECS: i64 = 300;

#[derive(Clone, Debug)]
pub struct AuthConfig {
    /// Application (client) ID of a public-client app registration.
    pub client_id: String,
    /// `common`, `consumers`, `organizations` or a tenant ID.
    pub tenant: String,
    /// Usually `https://login.microsoftonline.com`.
    pub authority: String,
}

impl AuthConfig {
    fn endpoint(&self, what: &str) -> String {
        format!(
            "{}/{}/oauth2/v2.0/{}",
            self.authority.trim_end_matches('/'),
            self.tenant,
            what
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

/// An OAuth error response such as `invalid_grant` or `authorization_pending`.
#[derive(Debug, thiserror::Error)]
#[error("{code}: {description}")]
pub struct OAuthFailure {
    pub code: String,
    pub description: String,
}

#[derive(Deserialize)]
struct OAuthErrorBody {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

async fn token_request(
    http: &reqwest::Client,
    cfg: &AuthConfig,
    form: &[(&str, &str)],
) -> Result<Tokens> {
    let resp = http
        .post(cfg.endpoint("token"))
        .form(form)
        .send()
        .await
        .context("contacting the Microsoft sign-in service")?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        if let Ok(e) = serde_json::from_str::<OAuthErrorBody>(&body) {
            return Err(OAuthFailure {
                code: e.error,
                description: e.error_description.unwrap_or_default(),
            }
            .into());
        }
        bail!("token endpoint returned {status}: {body}");
    }
    let t: TokenResponse = serde_json::from_str(&body).context("parsing token response")?;
    Ok(Tokens {
        access_token: t.access_token,
        refresh_token: t.refresh_token,
        expires_at: now() + t.expires_in.unwrap_or(3600),
    })
}

fn random_urlsafe(bytes: usize) -> Result<String> {
    let mut buf = vec![0u8; bytes];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// Persistent, refreshing token cache (a 0600 JSON file).
pub struct TokenStore {
    path: PathBuf,
    cfg: std::sync::Mutex<AuthConfig>,
    tokens: tokio::sync::Mutex<Option<Tokens>>,
    http: reqwest::Client,
}

impl TokenStore {
    pub fn open(path: impl Into<PathBuf>, cfg: AuthConfig) -> Self {
        let path = path.into();
        let tokens = read_tokens(&path);
        Self {
            path,
            cfg: std::sync::Mutex::new(cfg),
            tokens: tokio::sync::Mutex::new(tokens),
            http: reqwest::Client::new(),
        }
    }

    pub fn set_config(&self, cfg: AuthConfig) {
        *self.cfg.lock().unwrap() = cfg;
    }

    fn config(&self) -> AuthConfig {
        self.cfg.lock().unwrap().clone()
    }

    pub async fn is_signed_in(&self) -> bool {
        self.tokens.lock().await.is_some()
    }

    /// Re-read the file, e.g. after `axentra-onedrive login` ran in another process.
    pub async fn reload(&self) {
        *self.tokens.lock().await = read_tokens(&self.path);
    }

    pub async fn set(&self, tokens: Tokens) -> Result<()> {
        write_tokens(&self.path, &tokens)?;
        *self.tokens.lock().await = Some(tokens);
        Ok(())
    }

    pub async fn clear(&self) -> Result<()> {
        *self.tokens.lock().await = None;
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Force a refresh on the next request after `rejected` drew an unexpected
    /// 401. A no-op once a newer token replaced it, so that a burst of
    /// requests sent with the old token refreshes only once.
    pub async fn invalidate(&self, rejected: &str) {
        if let Some(t) = self.tokens.lock().await.as_mut() {
            if t.access_token == rejected {
                t.expires_at = 0;
            }
        }
    }

    pub async fn access_token(&self) -> Result<String, GraphError> {
        let mut guard = self.tokens.lock().await;
        let Some(tokens) = guard.as_ref() else {
            return Err(GraphError::NotSignedIn);
        };
        if tokens.expires_at - EXPIRY_MARGIN_SECS > now() {
            return Ok(tokens.access_token.clone());
        }
        let Some(refresh) = tokens.refresh_token.clone() else {
            *guard = None;
            return Err(GraphError::NotSignedIn);
        };
        let cfg = self.config();
        let result = token_request(
            &self.http,
            &cfg,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &cfg.client_id),
                ("refresh_token", &refresh),
                ("scope", SCOPES),
            ],
        )
        .await;
        match result {
            Ok(mut fresh) => {
                if fresh.refresh_token.is_none() {
                    fresh.refresh_token = Some(refresh);
                }
                // Keep the new tokens even if saving them fails (a full disk, say):
                // dropping them would mean another refresh for every request. The
                // older refresh token in the file still works, and the next
                // refresh tries to save again.
                if let Err(e) = write_tokens(&self.path, &fresh) {
                    tracing::warn!("couldn't save the refreshed sign-in: {e:#}");
                }
                let access = fresh.access_token.clone();
                *guard = Some(fresh);
                Ok(access)
            }
            Err(e) => match e.downcast_ref::<OAuthFailure>() {
                // The refresh token was revoked or expired: the user must sign in again.
                Some(f) if f.code == "invalid_grant" || f.code == "interaction_required" => {
                    *guard = None;
                    let _ = std::fs::remove_file(&self.path);
                    Err(GraphError::NotSignedIn)
                }
                // No network yet (for example just after boot) is "offline", not a failure.
                _ => match e.downcast::<reqwest::Error>() {
                    Ok(net) => Err(GraphError::Network(net)),
                    Err(e) => Err(GraphError::Other(format!("refreshing sign-in: {e:#}"))),
                },
            },
        }
    }
}

fn read_tokens(path: &Path) -> Option<Tokens> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_tokens(path: &Path, tokens: &Tokens) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(serde_json::to_string_pretty(tokens)?.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

const SIGNED_IN_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Signed in</title>\
<body style=\"font-family:system-ui,sans-serif;display:grid;place-items:center;height:90vh;color:#1b1b1b\">\
<div style=\"text-align:center\"><h2 style=\"font-weight:600\">You're signed in</h2>\
<p>Axentra OneDrive is setting up your files. You can close this tab.</p></div>";

const FAILED_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Sign-in failed</title>\
<body style=\"font-family:system-ui,sans-serif;display:grid;place-items:center;height:90vh\">\
<div style=\"text-align:center\"><h2>Sign-in didn't complete</h2><p>Return to the terminal for details.</p></div>";

async fn respond(sock: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = sock.write_all(resp.as_bytes()).await;
    let _ = sock.shutdown().await;
}

/// Browser sign-in. `open_url` is called with the authorize URL to show.
pub async fn login_browser(cfg: &AuthConfig, open_url: impl FnOnce(&str)) -> Result<Tokens> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let redirect = format!("http://localhost:{port}");
    let verifier = random_urlsafe(48)?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_urlsafe(16)?;

    let mut url = url::Url::parse(&cfg.endpoint("authorize"))?;
    url.query_pairs_mut()
        .append_pair("client_id", &cfg.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_mode", "query")
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("prompt", "select_account");
    open_url(url.as_str());

    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    loop {
        let (mut sock, _) = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| anyhow!("timed out waiting for the browser sign-in"))??;
        let mut buf = vec![0u8; 16 * 1024];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);
        let target = request
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/");
        let parsed = url::Url::parse(&format!("http://localhost{target}"))?;
        let query: HashMap<String, String> = parsed.query_pairs().into_owned().collect();

        if let Some(code) = query.get("code") {
            if query.get("state") != Some(&state) {
                respond(&mut sock, "400 Bad Request", FAILED_PAGE).await;
                bail!("sign-in response had the wrong state parameter");
            }
            let tokens = token_request(
                &reqwest::Client::new(),
                cfg,
                &[
                    ("grant_type", "authorization_code"),
                    ("client_id", &cfg.client_id),
                    ("code", code),
                    ("redirect_uri", &redirect),
                    ("code_verifier", &verifier),
                    ("scope", SCOPES),
                ],
            )
            .await;
            match &tokens {
                Ok(_) => respond(&mut sock, "200 OK", SIGNED_IN_PAGE).await,
                Err(_) => respond(&mut sock, "200 OK", FAILED_PAGE).await,
            }
            return tokens;
        }
        if let Some(err) = query.get("error") {
            respond(&mut sock, "200 OK", FAILED_PAGE).await;
            let desc = query.get("error_description").cloned().unwrap_or_default();
            bail!("sign-in failed: {err}: {desc}");
        }
        // Favicon and other stray requests.
        respond(&mut sock, "404 Not Found", "").await;
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub user_code: String,
    pub device_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default)]
    pub message: String,
}

fn default_interval() -> u64 {
    5
}

/// Device-code sign-in. `show` receives the code the user must enter.
pub async fn login_device_code(cfg: &AuthConfig, show: impl FnOnce(&DeviceCode)) -> Result<Tokens> {
    let http = reqwest::Client::new();
    let resp = http
        .post(cfg.endpoint("devicecode"))
        .form(&[("client_id", cfg.client_id.as_str()), ("scope", SCOPES)])
        .send()
        .await?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("device code request failed: {body}");
    }
    let code: DeviceCode = resp.json().await?;
    show(&code);
    let mut interval = code.interval.max(1);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(code.expires_in);
    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if tokio::time::Instant::now() > deadline {
            bail!("the device code expired before sign-in finished");
        }
        let result = token_request(
            &http,
            cfg,
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", &cfg.client_id),
                ("device_code", &code.device_code),
            ],
        )
        .await;
        match result {
            Ok(t) => return Ok(t),
            Err(e) => match e.downcast_ref::<OAuthFailure>().map(|f| f.code.as_str()) {
                Some("authorization_pending") => continue,
                Some("slow_down") => interval += 5,
                _ => return Err(e),
            },
        }
    }
}
