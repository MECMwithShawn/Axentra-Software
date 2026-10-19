//! OAuth 2.0 for the Microsoft identity platform (§3) and Google (§3.7):
//! authorization code + PKCE through a loopback redirect, the device code
//! flow as a fallback, and refresh. Tokens never touch the store or logs.

pub mod device;
pub mod loopback;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{SyncError, SyncResult, copy, from_reqwest};
use crate::secrets::Secret;

/// Sign-in + Graph (§3.3).
pub const GRAPH_SCOPES: &str =
    "openid profile offline_access User.Read Mail.ReadWrite Mail.Send Calendars.ReadWrite Contacts.ReadWrite Tasks.ReadWrite";
/// Graph scopes for refresh requests (one resource per token request).
pub const GRAPH_REFRESH_SCOPES: &str =
    "offline_access User.Read Mail.ReadWrite Mail.Send Calendars.ReadWrite Contacts.ReadWrite Tasks.ReadWrite";
/// IMAP / SMTP fallback for Microsoft accounts (fully qualified, §3.3).
pub const MS_IMAP_SCOPES: &str = "offline_access https://outlook.office.com/IMAP.AccessAsUser.All";
pub const MS_SMTP_SCOPES: &str = "offline_access https://outlook.office.com/SMTP.Send";
/// Gmail over IMAP / SMTP with XOAUTH2.
pub const GOOGLE_SCOPES: &str = "openid email https://mail.google.com/";

/// Tenant id of personal Microsoft accounts (the `tid` claim of MSA tokens).
pub const MSA_TENANT: &str = "9188040d-6c67-4c5b-b112-36a304b66dad";

/// One identity provider + app registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthClient {
    pub authorize_url: String,
    pub token_url: String,
    pub device_url: Option<String>,
    pub client_id: String,
    /// Google issues one to desktop clients; it is not confidential (§3.7).
    pub client_secret: Option<String>,
    pub microsoft: bool,
}

impl OAuthClient {
    /// `tenant` is `common`, `organizations`, `consumers` or a tenant GUID (§3.2).
    pub fn microsoft(tenant: &str, client_id: &str) -> Self {
        let base = format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0");
        OAuthClient {
            authorize_url: format!("{base}/authorize"),
            token_url: format!("{base}/token"),
            device_url: Some(format!("{base}/devicecode")),
            client_id: client_id.trim().to_string(),
            client_secret: None,
            microsoft: true,
        }
    }

    pub fn google(client_id: &str, client_secret: Option<&str>) -> Self {
        OAuthClient {
            authorize_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            device_url: None,
            client_id: client_id.trim().to_string(),
            client_secret: client_secret.map(str::to_string),
            microsoft: false,
        }
    }
}

/// Microsoft client id from the account / dialog, else `AXENTRA_MS_CLIENT_ID`.
pub fn ms_client_id(configured: Option<&str>) -> Option<String> {
    configured
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| std::env::var("AXENTRA_MS_CLIENT_ID").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
}

/// Google desktop client (`AXENTRA_GOOGLE_CLIENT_ID` / `_SECRET`).
pub fn google_client() -> Option<OAuthClient> {
    let id = std::env::var("AXENTRA_GOOGLE_CLIENT_ID").ok().filter(|s| !s.trim().is_empty())?;
    let secret = std::env::var("AXENTRA_GOOGLE_CLIENT_SECRET").ok().filter(|s| !s.trim().is_empty());
    Some(OAuthClient::google(&id, secret.as_deref()))
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("the OS random number generator is available");
    buf
}

/// 32 random bytes, base64url without padding (43 characters).
pub fn random_token() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<32>())
}

/// `code_challenge` = base64url(SHA-256(verifier)) (RFC 7636 S256).
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[derive(Clone, Debug)]
pub struct Pkce {
    pub verifier: Secret,
    pub challenge: String,
}

pub fn pkce() -> Pkce {
    let verifier = random_token();
    Pkce { challenge: challenge(&verifier), verifier: Secret::new(verifier) }
}

/// A token endpoint response.
#[derive(Clone, Debug)]
pub struct TokenSet {
    pub access_token: Secret,
    pub refresh_token: Option<Secret>,
    pub expires_at: DateTime<Utc>,
    pub id_token: Option<String>,
    pub scope: String,
}

/// Identity claims read (not validated) from an ID token, for display and
/// keying only (§3.4 step 9).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct IdClaims {
    pub oid: String,
    pub tid: String,
    pub preferred_username: String,
    pub name: String,
    pub email: String,
}

pub fn id_claims(id_token: &str) -> IdClaims {
    id_token
        .split('.')
        .nth(1)
        .and_then(|p| URL_SAFE_NO_PAD.decode(p.trim_end_matches('=')).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// The authorize URL for the browser (§3.4 step 4). Never combines
/// `login_hint` with `prompt=select_account`.
pub fn authorize_url(client: &OAuthClient, scopes: &str, redirect_uri: &str, state: &str, challenge: &str, login_hint: Option<&str>, select_account: bool) -> String {
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    q.append_pair("client_id", &client.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_mode", "query")
        .append_pair("scope", scopes)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    if select_account {
        q.append_pair("prompt", "select_account");
    } else if let Some(hint) = login_hint.filter(|h| !h.is_empty()) {
        q.append_pair("login_hint", hint);
    }
    if !client.microsoft {
        // Google only returns a refresh token for offline access.
        q.append_pair("access_type", "offline");
    }
    format!("{}?{}", client.authorize_url, q.finish())
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TokenError {
    pub error: String,
    pub error_description: String,
    pub error_codes: Vec<i64>,
    pub trace_id: String,
    pub correlation_id: String,
}

/// Classify a token endpoint error (§3.6, §2.4). The `error` string decides
/// the class; AADSTS numbers only pick friendlier copy.
pub fn classify_token_error(e: &TokenError) -> SyncError {
    let has = |code: i64| e.error_codes.contains(&code) || e.error_description.contains(&format!("AADSTS{code}"));
    if has(53003) {
        return SyncError::User(copy::CA_BLOCKED.into());
    }
    if has(65001) || has(90094) || has(90095) {
        return SyncError::User(copy::ADMIN_CONSENT.into());
    }
    if has(700016) {
        return SyncError::User(format!(
            "The application (client) ID wasn't found. Check the ID of your Microsoft Entra app registration. ({})",
            first_line(&e.error_description)
        ));
    }
    if has(7000218) {
        return SyncError::User("Turn on “Allow public client flows” in your Microsoft Entra app registration (see docs/ACCOUNTS.md).".into());
    }
    if has(50011) || has(500113) {
        return SyncError::User("The app registration's redirect URI doesn't match. Add http://localhost as a Mobile and desktop redirect URI (see docs/ACCOUNTS.md).".into());
    }
    match e.error.as_str() {
        "invalid_grant" | "interaction_required" | "login_required" => SyncError::Auth("Your sign-in has expired. Sign in again.".into()),
        "consent_required" | "invalid_scope" => SyncError::Auth("Axentra Outlook needs your permission again. Sign in again.".into()),
        "access_denied" => SyncError::User(copy::CANCELLED.into()),
        "temporarily_unavailable" | "server_error" => SyncError::Throttled { retry_after: None },
        "invalid_client" | "unauthorized_client" => {
            SyncError::User(format!("The app registration isn't set up for this sign-in: {}", first_line(&e.error_description)))
        }
        _ => SyncError::protocol(format!("Sign-in failed: {} {}", e.error, first_line(&e.error_description)).trim().to_string()),
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or_default().trim()
}

/// A failed token request: the error and, when the server sent one, its body.
pub(crate) type TokenFailure = Box<(SyncError, Option<TokenError>)>;

/// POST a token request; on error returns the parsed error body too.
pub(crate) async fn token_request(http: &reqwest::Client, client: &OAuthClient, params: &[(&str, &str)]) -> Result<TokenSet, TokenFailure> {
    let mut form: Vec<(&str, &str)> = vec![("client_id", client.client_id.as_str())];
    if let Some(secret) = &client.client_secret {
        form.push(("client_secret", secret));
    }
    form.extend_from_slice(params);
    let resp = http.post(&client.token_url).form(&form).send().await.map_err(|e| Box::new((from_reqwest(&e), None)))?;
    let status = resp.status();
    let body = resp.bytes().await.map_err(|e| Box::new((from_reqwest(&e), None)))?;
    if status.is_success() {
        let t: TokenResponse =
            serde_json::from_slice(&body).map_err(|_| Box::new((SyncError::protocol("The sign-in server sent an unexpected response."), None)))?;
        return Ok(TokenSet {
            access_token: Secret::new(t.access_token),
            refresh_token: t.refresh_token.map(Secret::new),
            expires_at: Utc::now() + Duration::seconds(t.expires_in.unwrap_or(3600)),
            id_token: t.id_token,
            scope: t.scope.unwrap_or_default(),
        });
    }
    let err: TokenError = serde_json::from_slice(&body).unwrap_or_else(|_| TokenError { error: format!("http_{}", status.as_u16()), ..Default::default() });
    if !err.trace_id.is_empty() {
        tracing::warn!("token endpoint error {} (trace_id {}, correlation_id {})", err.error, err.trace_id, err.correlation_id);
    }
    if status.as_u16() == 429 || status.is_server_error() {
        return Err(Box::new((SyncError::Throttled { retry_after: None }, Some(err))));
    }
    Err(Box::new((classify_token_error(&err), Some(err))))
}

/// Redeem an authorization code immediately (codes live about a minute).
pub async fn redeem_code(http: &reqwest::Client, client: &OAuthClient, code: &str, verifier: &Secret, redirect_uri: &str, scopes: &str) -> SyncResult<TokenSet> {
    token_request(
        http,
        client,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier.expose()),
            ("scope", scopes),
        ],
    )
    .await
    .map_err(|f| f.0)
}

/// Exchange a refresh token for a new access token for one resource.
pub async fn refresh(http: &reqwest::Client, client: &OAuthClient, refresh_token: &Secret, scopes: &str) -> SyncResult<TokenSet> {
    token_request(http, client, &[("grant_type", "refresh_token"), ("refresh_token", refresh_token.expose()), ("scope", scopes)])
        .await
        .map_err(|f| f.0)
}

/// Open a URL in the user's browser (xdg-open, which goes through the
/// OpenURI portal inside Flatpak).
pub fn open_in_browser(url: &str) -> SyncResult<()> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|_| SyncError::User("Couldn't open your web browser. Use “Copy sign-in link” and paste it into a browser.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636_example() {
        // RFC 7636 appendix B.
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let p = pkce();
        assert_eq!(p.verifier.expose().len(), 43);
        assert!(p.verifier.expose().chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_eq!(p.challenge, challenge(p.verifier.expose()));
        assert_ne!(random_token(), random_token());
    }

    #[test]
    fn authorize_url_shape() {
        let c = OAuthClient::microsoft("common", "11111111-2222-3333-4444-555555555555");
        let url = authorize_url(&c, GRAPH_SCOPES, "http://127.0.0.1:49152", "st", "ch", Some("ana@contoso.com"), false);
        assert!(url.starts_with("https://login.microsoftonline.com/common/oauth2/v2.0/authorize?client_id=11111111"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A49152&"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("login_hint=ana%40contoso.com"));
        assert!(url.contains("scope=openid+profile+offline_access+User.Read"));
        assert!(!url.contains("prompt="));
        let retry = authorize_url(&c, GRAPH_SCOPES, "http://127.0.0.1:1", "st", "ch", Some("ana@contoso.com"), true);
        assert!(retry.contains("prompt=select_account") && !retry.contains("login_hint"));
        let g = OAuthClient::google("id.apps.googleusercontent.com", Some("s"));
        assert!(authorize_url(&g, GOOGLE_SCOPES, "http://127.0.0.1:1", "s", "c", None, false).contains("access_type=offline"));
    }

    #[test]
    fn token_errors_map_to_states_and_copy() {
        let e = |error: &str, codes: Vec<i64>| TokenError { error: error.into(), error_codes: codes, ..Default::default() };
        assert!(matches!(classify_token_error(&e("invalid_grant", vec![70008])), SyncError::Auth(_)));
        assert!(matches!(classify_token_error(&e("interaction_required", vec![50076])), SyncError::Auth(_)));
        assert_eq!(classify_token_error(&e("invalid_grant", vec![53003])).to_string(), copy::CA_BLOCKED);
        assert_eq!(classify_token_error(&e("consent_required", vec![65001])).to_string(), copy::ADMIN_CONSENT);
        assert_eq!(classify_token_error(&e("access_denied", vec![65004])).to_string(), copy::CANCELLED);
        assert!(matches!(classify_token_error(&e("temporarily_unavailable", vec![])), SyncError::Throttled { .. }));
        assert!(matches!(classify_token_error(&e("invalid_client", vec![])), SyncError::User(_)));
    }

    #[test]
    fn id_token_claims_are_read_without_validation() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"oid":"o-1","tid":"t-1","preferred_username":"ana@contoso.com","name":"Ana Ruiz"}"#);
        let claims = id_claims(&format!("eyJhbGciOiJub25lIn0.{payload}.sig"));
        assert_eq!(claims.preferred_username, "ana@contoso.com");
        assert_eq!((claims.oid.as_str(), claims.tid.as_str(), claims.name.as_str()), ("o-1", "t-1", "Ana Ruiz"));
        assert_eq!(id_claims("garbage"), IdClaims::default());
    }
}
