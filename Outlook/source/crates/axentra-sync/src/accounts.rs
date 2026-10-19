//! Adding, re-authenticating and removing accounts: the backend of
//! Settings > Accounts > Email accounts and the Add account dialog (§2).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use axentra_core::{Account, AccountId, AccountKind};
use chrono::{DateTime, Utc};
use tokio::task::{AbortHandle, JoinHandle};

use crate::config::{AccountConfig, AccountState, AuthMethod, Provider, ServerConfig, is_valid_email, preset};
use crate::engine::{Inner, Resource, Scope};
use crate::error::{SyncError, SyncResult, copy};
use crate::graph::client::StaticToken;
use crate::graph::{Graph, me_from};
use crate::imap::{self, Credential};
use crate::oauth::loopback::BrowserSignIn;
use crate::oauth::{self, IdClaims, OAuthClient, TokenSet};
use crate::secrets::{Secret, SecretKind};
use crate::smtp::{self, SmtpAuth};

/// Account badge colours, assigned round-robin.
const ACCOUNT_COLORS: &[&str] = &["#0f6cbd", "#038387", "#8764b8", "#ca5010", "#498205", "#c4314b", "#4f6bed", "#986f0b"];

/// The manual IMAP form (§2.2 step 3e) or a preset filled in.
#[derive(Clone, Debug, Default)]
pub struct ImapSetup {
    pub display_name: String,
    pub email: String,
    pub password: Secret,
    pub provider: Provider,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    pub smtp_same_auth: bool,
    pub smtp_password: Option<Secret>,
}

impl ImapSetup {
    fn validate(&self) -> SyncResult<()> {
        if !is_valid_email(&self.email) {
            return Err(SyncError::User("Enter a valid email address.".into()));
        }
        if self.password.is_empty() {
            return Err(SyncError::User("Enter your password.".into()));
        }
        if self.imap.host.trim().is_empty() || self.imap.port == 0 {
            return Err(SyncError::User("Enter the incoming mail (IMAP) server and port.".into()));
        }
        if self.smtp.host.trim().is_empty() || self.smtp.port == 0 {
            return Err(SyncError::User("Enter the outgoing mail (SMTP) server and port.".into()));
        }
        Ok(())
    }

    fn imap_user(&self) -> &str {
        if self.imap.username.trim().is_empty() { self.email.trim() } else { self.imap.username.trim() }
    }

    fn smtp_auth(&self) -> SmtpAuth {
        let user = if self.smtp.username.trim().is_empty() { self.imap_user().to_string() } else { self.smtp.username.trim().to_string() };
        let password = if self.smtp_same_auth { self.password.clone() } else { self.smtp_password.clone().unwrap_or_else(|| self.password.clone()) };
        SmtpAuth::Password { user, password }
    }
}

fn duplicate(inner: &Inner, email: &str) -> SyncResult<Option<Account>> {
    Ok(inner.store.accounts()?.into_iter().find(|a| a.kind != AccountKind::Local && a.email.eq_ignore_ascii_case(email.trim())))
}

fn next_color(inner: &Inner) -> String {
    let n = inner.store.accounts().map(|a| a.len()).unwrap_or(0);
    ACCOUNT_COLORS[n % ACCOUNT_COLORS.len()].to_string()
}

/// Sign in to IMAP and SMTP without saving anything ("Test connection").
pub async fn test_imap(setup: &ImapSetup) -> SyncResult<Vec<String>> {
    setup.validate()?;
    let mut session = imap::open(&setup.imap, setup.imap_user(), &Credential::Password(setup.password.clone())).await?;
    let mut caps: Vec<String> = session.caps.iter().cloned().collect();
    caps.sort();
    let _ = session.session.logout().await;
    smtp::test(&setup.smtp, &setup.smtp_auth()).await?;
    Ok(caps)
}

/// Test, store the password in the keyring, create the account and start syncing.
pub async fn add_imap(inner: &Arc<Inner>, setup: ImapSetup) -> SyncResult<AccountId> {
    if duplicate(inner, &setup.email)?.is_some() {
        return Err(SyncError::User(copy::ALREADY_PRESENT.into()));
    }
    let caps = test_imap(&setup).await?;
    let email = setup.email.trim().to_string();
    let display = if setup.display_name.trim().is_empty() { email.split('@').next().unwrap_or_default().to_string() } else { setup.display_name.trim().to_string() };
    let mut cfg = AccountConfig {
        uuid: uuid::Uuid::new_v4().to_string(),
        provider: setup.provider,
        auth: AuthMethod::Password,
        imap: Some(setup.imap.clone()),
        smtp: Some(setup.smtp.clone()),
        smtp_same_auth: setup.smtp_same_auth,
        capabilities: caps,
        added_at: Some(Utc::now()),
        ..Default::default()
    };
    if setup.provider == Provider::Google {
        cfg.sent_copy_policy = crate::config::SentCopyPolicy::Server;
    }
    let mut account = Account { kind: AccountKind::Imap, display_name: display, email, color: next_color(inner), ..Default::default() };
    cfg.store_into(&mut account);
    // Secrets first: no keyring, no account (never plain text).
    inner.keyring.set(&Inner::secret_key(&account, &cfg, SecretKind::Password), &setup.password).await?;
    if !setup.smtp_same_auth
        && let Some(p) = &setup.smtp_password
    {
        inner.keyring.set(&Inner::secret_key(&account, &cfg, SecretKind::SmtpPassword), p).await?;
    }
    account.sort_order = inner.store.next_account_sort_order()?;
    let id = inner.store.add_account(&mut account)?;
    inner.notifier.bump();
    inner.request(id, Scope::Full);
    Ok(id)
}

/// Replace the saved password of an IMAP account (after "Sign in" on a
/// `needs_reauth` account) and resume syncing.
pub async fn update_password(inner: &Arc<Inner>, id: AccountId, password: Secret) -> SyncResult<()> {
    let account = inner.store.account(id)?.ok_or_else(|| SyncError::User("That account no longer exists.".into()))?;
    let cfg = AccountConfig::of(&account);
    let server = cfg.imap.clone().ok_or_else(|| SyncError::User("The incoming mail server isn't set up.".into()))?;
    let mut session = imap::open(&server, cfg.imap_username(&account.email), &Credential::Password(password.clone())).await?;
    let _ = session.session.logout().await;
    inner.keyring.set(&Inner::secret_key(&account, &cfg, SecretKind::Password), &password).await?;
    inner.store.update_account_with(id, |a| {
        let mut c = AccountConfig::of(a);
        c.state = AccountState::Ok;
        c.state_detail = None;
        a.sync_error = None;
        c.store_into(a);
    })?;
    inner.notifier.bump();
    inner.request(id, Scope::Full);
    Ok(())
}

// ---------------------------------------------------------------- OAuth sign-in

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInKind {
    /// `tenant`: `common`, `organizations` or `consumers` (§3.2).
    Microsoft { tenant: String, client_id: Option<String> },
    /// Gmail over IMAP with XOAUTH2 (needs AXENTRA_GOOGLE_CLIENT_ID).
    Google,
    /// Sign in again to an existing account.
    Reauth(AccountId),
}

#[derive(Clone, Debug)]
pub struct SignInRequest {
    /// The address the user typed (login hint).
    pub email: String,
    pub kind: SignInKind,
    /// "Try again" after an identity mismatch: show the account picker.
    pub select_account: bool,
}

/// The result of a completed browser / device sign-in, before the account
/// is created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedIn {
    pub flow: u64,
    pub email: String,
    pub display_name: String,
    /// The identity matches what the user typed (§2.4 "different identity").
    pub matches: bool,
}

/// A running sign-in; abort it on Cancel.
pub struct SignInTask {
    pub abort: AbortHandle,
    pub handle: JoinHandle<SyncResult<SignedIn>>,
}

impl SignInTask {
    pub async fn wait(self) -> SyncResult<SignedIn> {
        self.handle.await.unwrap_or(Err(SyncError::Cancelled))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: DateTime<Utc>,
}

struct Pending {
    tokens: TokenSet,
    email: String,
    display_name: String,
    claims: IdClaims,
    client: OAuthClient,
    kind: SignInKind,
    scopes: String,
    created: Instant,
}

fn pending() -> &'static Mutex<HashMap<u64, Pending>> {
    static P: OnceLock<Mutex<HashMap<u64, Pending>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

fn next_flow() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn client_for(inner: &Inner, req: &SignInRequest) -> SyncResult<(OAuthClient, String)> {
    match &req.kind {
        SignInKind::Microsoft { tenant, client_id } => {
            let id = oauth::ms_client_id(client_id.as_deref()).ok_or_else(|| SyncError::User(copy::NO_CLIENT_ID.into()))?;
            Ok((OAuthClient::microsoft(tenant, &id), oauth::GRAPH_SCOPES.into()))
        }
        SignInKind::Google => Ok((
            oauth::google_client().ok_or_else(|| SyncError::User("Google sign-in isn't set up. Use an app password instead.".into()))?,
            oauth::GOOGLE_SCOPES.into(),
        )),
        SignInKind::Reauth(id) => {
            let account = inner.store.account(*id)?.ok_or_else(|| SyncError::User("That account no longer exists.".into()))?;
            let cfg = AccountConfig::of(&account);
            let mut client = Inner::oauth_client(&cfg)?;
            if client.microsoft {
                // Interactive sign-in uses the organizations / consumers authority;
                // refreshes go to the stored tenant.
                let tenant = if cfg.provider == Provider::MicrosoftPersonal { "consumers" } else { cfg.ms_tenant.as_deref().unwrap_or("organizations") };
                client = OAuthClient::microsoft(tenant, &client.client_id);
            }
            let scopes = if cfg.auth == AuthMethod::Xoauth2Google { oauth::GOOGLE_SCOPES } else { oauth::GRAPH_SCOPES };
            Ok((client, scopes.into()))
        }
    }
}

/// After tokens arrive: identity, mailbox probe, park the tokens.
async fn complete(inner: Arc<Inner>, client: OAuthClient, scopes: String, req: SignInRequest, tokens: TokenSet) -> SyncResult<SignedIn> {
    let claims = tokens.id_token.as_deref().map(oauth::id_claims).unwrap_or_default();
    let (email, display_name) = if client.microsoft {
        let graph = Graph::new(inner.http.clone(), Arc::new(StaticToken(tokens.access_token.clone())));
        let me = me_from(&graph.get("/me?$select=id,displayName,mail,userPrincipalName").await?);
        let email = if me.email.is_empty() { claims.preferred_username.clone() } else { me.email.clone() };
        match graph.get("/me/mailFolders/inbox?$select=id").await {
            Ok(_) => {}
            Err(SyncError::User(_)) if claims.tid == oauth::MSA_TENANT => return Err(SyncError::User(copy::NO_OUTLOOK_COM_MAILBOX.into())),
            Err(SyncError::User(_)) => return Err(SyncError::User(copy::no_mailbox(&email))),
            Err(e) => return Err(e),
        }
        let name = if me.display_name.is_empty() { claims.name.clone() } else { me.display_name };
        (email, name)
    } else {
        let email = if claims.email.is_empty() { req.email.clone() } else { claims.email.clone() };
        (email.clone(), if claims.name.is_empty() { email.split('@').next().unwrap_or_default().to_string() } else { claims.name.clone() })
    };
    let expected = match &req.kind {
        SignInKind::Reauth(id) => inner.store.account(*id)?.map(|a| a.email).unwrap_or_default(),
        _ => req.email.clone(),
    };
    let matches = expected.trim().is_empty() || expected.trim().eq_ignore_ascii_case(email.trim());
    let flow = next_flow();
    let mut p = pending().lock().unwrap_or_else(|e| e.into_inner());
    p.retain(|_, v| v.created.elapsed().as_secs() < 15 * 60);
    p.insert(flow, Pending { tokens, email: email.clone(), display_name: display_name.clone(), claims, client, kind: req.kind, scopes, created: Instant::now() });
    Ok(SignedIn { flow, email, display_name, matches })
}

/// Start a browser sign-in. Returns the authorize URL (for "Copy sign-in
/// link") and the task that finishes when the browser comes back.
pub async fn start_browser(inner: &Arc<Inner>, req: SignInRequest) -> SyncResult<(String, SignInTask)> {
    let (client, scopes) = client_for(inner, &req)?;
    let hint = (!req.email.trim().is_empty()).then(|| req.email.trim().to_string());
    let flow = BrowserSignIn::begin(client.clone(), &scopes, hint.as_deref(), req.select_account).await?;
    let url = flow.auth_url.clone();
    oauth::open_in_browser(&url)?;
    let inner2 = inner.clone();
    let handle = tokio::spawn(async move {
        let tokens = flow.finish(&inner2.http).await?;
        complete(inner2, client, scopes, req, tokens).await
    });
    Ok((url, SignInTask { abort: handle.abort_handle(), handle }))
}

/// Start the device code flow (Microsoft only).
pub async fn start_device(inner: &Arc<Inner>, req: SignInRequest) -> SyncResult<(DeviceCodeInfo, SignInTask)> {
    let (client, scopes) = client_for(inner, &req)?;
    let code = oauth::device::start(&inner.http, &client, &scopes).await?;
    let info = DeviceCodeInfo { user_code: code.user_code.clone(), verification_uri: code.verification_uri.clone(), expires_at: code.expires_at };
    let inner2 = inner.clone();
    let handle = tokio::spawn(async move {
        let tokens = oauth::device::poll(&inner2.http, &client, &code).await?;
        complete(inner2, client, scopes, req, tokens).await
    });
    Ok((info, SignInTask { abort: handle.abort_handle(), handle }))
}

/// Create the account (or repair the re-authenticated one) from a finished sign-in.
pub async fn finish(inner: &Arc<Inner>, flow: u64) -> SyncResult<AccountId> {
    let p = pending()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&flow)
        .ok_or_else(|| SyncError::User(copy::SIGNIN_TIMEOUT.into()))?;
    let refresh = p.tokens.refresh_token.clone().ok_or_else(|| SyncError::User("The sign-in didn't grant offline access. Please try again.".into()))?;
    if let SignInKind::Reauth(id) = p.kind {
        let account = inner.store.account(id)?.ok_or_else(|| SyncError::User("That account no longer exists.".into()))?;
        if !account.email.eq_ignore_ascii_case(&p.email) {
            return Err(SyncError::User(format!("You signed in as {}. Sign in as {} to fix this account.", p.email, account.email)));
        }
        let cfg = AccountConfig::of(&account);
        inner.save_refresh_token(&account, &cfg, &refresh, &p.scopes).await?;
        inner.store.update_account_with(id, |a| {
            let mut c = AccountConfig::of(a);
            c.state = AccountState::Ok;
            c.state_detail = None;
            a.sync_error = None;
            c.store_into(a);
        })?;
        let resource = if cfg.auth == AuthMethod::OauthGraph { Resource::Graph } else { Resource::Imap };
        inner.cache_token(id, resource, p.tokens.access_token.clone(), p.tokens.expires_at);
        inner.notifier.bump();
        inner.request(id, Scope::Full);
        return Ok(id);
    }
    if duplicate(inner, &p.email)?.is_some() {
        return Err(SyncError::User(copy::ALREADY_PRESENT.into()));
    }
    let mut cfg = AccountConfig { uuid: uuid::Uuid::new_v4().to_string(), added_at: Some(Utc::now()), ..Default::default() };
    let kind = if p.client.microsoft {
        let personal = p.claims.tid == oauth::MSA_TENANT;
        cfg.provider = if personal { Provider::MicrosoftPersonal } else { Provider::MicrosoftWork };
        cfg.auth = AuthMethod::OauthGraph;
        cfg.ms_client_id = Some(p.client.client_id.clone());
        cfg.ms_tenant = Some(if personal || p.claims.tid.is_empty() { "consumers".to_string() } else { p.claims.tid.clone() });
        if p.claims.tid.is_empty() {
            cfg.ms_tenant = Some("common".into());
        }
        cfg.ms_oid = Some(p.claims.oid.clone()).filter(|s| !s.is_empty());
        AccountKind::Microsoft
    } else {
        let gmail = preset(Provider::Google).expect("Gmail preset");
        cfg.provider = Provider::Google;
        cfg.auth = AuthMethod::Xoauth2Google;
        cfg.google_client_id = Some(p.client.client_id.clone());
        cfg.imap = Some(gmail.imap);
        cfg.smtp = Some(gmail.smtp);
        cfg.sent_copy_policy = crate::config::SentCopyPolicy::Server;
        AccountKind::Imap
    };
    let mut account = Account { kind, display_name: p.display_name.clone(), email: p.email.clone(), color: next_color(inner), ..Default::default() };
    cfg.store_into(&mut account);
    inner.save_refresh_token(&account, &cfg, &refresh, &p.scopes).await?;
    account.sort_order = inner.store.next_account_sort_order()?;
    let id = inner.store.add_account(&mut account)?;
    let resource = if kind == AccountKind::Microsoft { Resource::Graph } else { Resource::Imap };
    inner.cache_token(id, resource, p.tokens.access_token.clone(), p.tokens.expires_at);
    inner.notifier.bump();
    inner.request(id, Scope::Full);
    Ok(id)
}

/// Remove an account, its keyring items and everything stored for it.
pub async fn remove(inner: &Arc<Inner>, id: AccountId) -> SyncResult<()> {
    let Some(account) = inner.store.account(id)? else { return Ok(()) };
    inner.forget_account(&account).await
}

/// "Unlock" for the `keyring_locked` state: prompt, then sync.
pub async fn unlock_keyring(inner: &Arc<Inner>, id: AccountId) -> SyncResult<()> {
    let Some(account) = inner.store.account(id)? else { return Ok(()) };
    let cfg = AccountConfig::of(&account);
    inner.keyring.unlock(&cfg.uuid).await?;
    inner.request(id, Scope::Full);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Security;
    use crate::notify::Notifier;
    use crate::secrets::Keyring;

    fn inner() -> Arc<Inner> {
        let store = Arc::new(axentra_core::Store::open_in_memory().unwrap());
        Inner::new(store, Notifier::default(), Arc::default(), Keyring::memory())
    }

    #[tokio::test]
    async fn imap_setup_validation_and_unreachable_servers() {
        let inner = inner();
        let mut setup = ImapSetup { email: "ana@example".into(), ..Default::default() };
        assert!(matches!(test_imap(&setup).await, Err(SyncError::User(_))));
        setup.email = "ana@example.org".into();
        setup.password = Secret::new("pw");
        setup.imap = ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::Tls, username: String::new() };
        setup.smtp = ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::Tls, username: String::new() };
        let err = add_imap(&inner, setup).await.unwrap_err();
        assert!(matches!(err, SyncError::Offline(_)), "{err:?}");
        assert!(inner.store.accounts().unwrap().is_empty(), "nothing saved when the test fails");
    }

    #[tokio::test]
    async fn finishing_an_unknown_flow_is_a_timeout() {
        let err = finish(&inner(), 987_654).await.unwrap_err();
        assert_eq!(err.to_string(), copy::SIGNIN_TIMEOUT);
    }
}
