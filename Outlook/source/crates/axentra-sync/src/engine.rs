//! The engine: per-account scheduling, sync passes for IMAP and Graph
//! accounts, OAuth token cache, Outbox delivery and live status.
//!
//! Cadence (estimates from §4.12 / §5.5): Graph quick pass (Inbox + push)
//! every 60 s, IMAP every 2 min without IDLE; a full pass (folders, all
//! mail, calendar, To Do) every 5 min; contacts every 30 min; Outbox and
//! local edits are checked every 5 s.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axentra_core::{Account, AccountId, AccountKind, FolderRole, Store};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use serde_json::json;
use tokio::task::JoinHandle;

use crate::config::{AccountConfig, AccountState, AuthMethod, Provider, SentCopyPolicy};
use crate::error::{KeyringError, SyncError, SyncResult, copy};
use crate::graph::calendar::GraphCalendar;
use crate::graph::mail::GraphMail;
use crate::graph::todo::GraphTodo;
use crate::graph::{Graph, TokenSource, contacts};
use crate::imap::sync::MailSync;
use crate::imap::{self, Credential};
use crate::notify::Notifier;
use crate::oauth::{self, OAuthClient};
use crate::outbox;
use crate::secrets::{Keyring, Secret, SecretKey, SecretKind};
use crate::smtp::{self, SmtpAuth};
use crate::{AccountSyncInfo, Notice, SyncStatus};

pub(crate) const TICK: Duration = Duration::from_secs(5);
const FULL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const GRAPH_QUICK: Duration = Duration::from_secs(60);
const IMAP_POLL: Duration = Duration::from_secs(120);
const IMAP_IDLE_QUICK: Duration = Duration::from_secs(15 * 60);
const CONTACTS_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// Refresh tokens this long before they expire (estimate, §3.6).
const TOKEN_MARGIN: chrono::Duration = chrono::Duration::minutes(5);

/// How much a pass does, smallest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// Push local edits only (no reads).
    Push,
    /// Deliver the Outbox.
    Outbox,
    /// Inbox + push.
    Quick,
    /// Everything.
    Full,
}

/// Which token a request needs (one resource per token, §3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resource {
    Graph,
    Imap,
    Smtp,
}

/// An access token in memory with its expiry (never persisted, §3.6).
type CachedToken = (Secret, DateTime<Utc>);

#[derive(Default)]
struct Sched {
    running: bool,
    queued: Option<Scope>,
    last_quick: Option<Instant>,
    last_full: Option<Instant>,
    last_contacts: Option<Instant>,
    retry_at: Option<Instant>,
    failures: u32,
    idle: Option<JoinHandle<()>>,
    idle_unsupported: bool,
    progress: Option<String>,
    first_pass_done: bool,
}

/// Outbox retry bookkeeping (1, 5, 15, 60 min, §5.9).
#[derive(Clone, Copy)]
struct OutboxRetry {
    attempts: u32,
    next: Instant,
}

pub struct Inner {
    pub store: Arc<Store>,
    pub notifier: Notifier,
    pub keyring: Keyring,
    pub http: reqwest::Client,
    status: Arc<Mutex<SyncStatus>>,
    sched: Mutex<HashMap<AccountId, Sched>>,
    tokens: Mutex<HashMap<(AccountId, Resource), CachedToken>>,
    refresh_locks: Mutex<HashMap<AccountId, Arc<tokio::sync::Mutex<()>>>>,
    outbox_retry: Mutex<HashMap<i64, OutboxRetry>>,
    notices: Mutex<Vec<Notice>>,
    last_total_changes: Mutex<u64>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("AxentraOutlook/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default()
}

impl Inner {
    pub fn new(store: Arc<Store>, notifier: Notifier, status: Arc<Mutex<SyncStatus>>, keyring: Keyring) -> Arc<Self> {
        Arc::new(Inner {
            store,
            notifier,
            keyring,
            http: http_client(),
            status,
            sched: Mutex::new(HashMap::new()),
            tokens: Mutex::new(HashMap::new()),
            refresh_locks: Mutex::new(HashMap::new()),
            outbox_retry: Mutex::new(HashMap::new()),
            notices: Mutex::new(Vec::new()),
            last_total_changes: Mutex::new(0),
        })
    }

    // ------------------------------------------------------------ status

    pub fn notice(&self, text: impl Into<String>, error: bool) {
        let mut n = lock(&self.notices);
        n.push(Notice { text: text.into(), error });
        let excess = n.len().saturating_sub(10);
        n.drain(..excess);
    }

    pub fn take_notices(&self) -> Vec<Notice> {
        std::mem::take(&mut *lock(&self.notices))
    }

    fn publish_status(&self) {
        let accounts = self.store.accounts().unwrap_or_default();
        let sched = lock(&self.sched);
        let now = Utc::now();
        let mut st = SyncStatus { running: sched.values().any(|s| s.running), ..Default::default() };
        for a in &accounts {
            let s = sched.get(&a.id);
            let syncing = s.is_some_and(|s| s.running);
            let line = if syncing { "Syncing…".to_string() } else { crate::config::status_line(a, now) };
            st.accounts.push((a.id, line.clone()));
            st.details.push(AccountSyncInfo {
                account_id: a.id,
                syncing,
                first_sync: syncing && a.last_sync_at.is_none(),
                progress: s.and_then(|s| s.progress.clone()),
                status_line: line,
                error: a.sync_error.clone(),
            });
            if st.last_error.is_none() {
                st.last_error = a.sync_error.clone();
            }
        }
        drop(sched);
        *lock(&self.status) = st;
    }

    fn set_progress(&self, id: AccountId, progress: Option<String>) {
        if let Some(s) = lock(&self.sched).get_mut(&id) {
            s.progress = progress;
        }
        self.publish_status();
    }

    /// Persist an account's health; bumps the UI only when it changed.
    fn record(&self, id: AccountId, result: &SyncResult<()>) {
        let mut changed = false;
        let _ = self.store.update_account_with(id, |a| {
            let mut cfg = AccountConfig::of(a);
            let before = (cfg.state, a.sync_error.clone());
            match result {
                Ok(()) => {
                    cfg.state = AccountState::Ok;
                    cfg.state_detail = None;
                    a.sync_error = None;
                    a.last_sync_at = Some(Utc::now());
                }
                Err(e) => {
                    cfg.state = e.account_state();
                    let reason = e.short_reason();
                    cfg.state_detail = Some(reason.clone());
                    a.sync_error = Some(reason);
                }
            }
            if before.0 != cfg.state {
                cfg.state_since = Some(Utc::now());
            }
            if before != (cfg.state, a.sync_error.clone()) {
                cfg.bar_dismissed = false;
                changed = true;
            }
            cfg.store_into(a);
        });
        if changed {
            self.notifier.bump();
        }
    }

    /// Clear states that can't be true after a restart.
    pub fn reset_transient_states(&self) {
        for a in self.store.accounts().unwrap_or_default() {
            let cfg = AccountConfig::of(&a);
            if matches!(cfg.state, AccountState::Syncing | AccountState::Throttled | AccountState::Offline) {
                let _ = self.store.update_account_with(a.id, |a| {
                    let mut c = AccountConfig::of(a);
                    c.state = AccountState::Ok;
                    c.store_into(a);
                });
            }
        }
        *lock(&self.last_total_changes) = self.store.total_changes();
        self.publish_status();
    }

    // ------------------------------------------------------------ secrets and tokens

    pub fn secret_key(account: &Account, cfg: &AccountConfig, kind: SecretKind) -> SecretKey {
        SecretKey { account_uuid: cfg.uuid.clone(), kind, provider: cfg.provider, username: account.email.clone() }
    }

    pub fn oauth_client(cfg: &AccountConfig) -> SyncResult<OAuthClient> {
        match cfg.auth {
            AuthMethod::Xoauth2Google => {
                let mut c = oauth::google_client().ok_or_else(|| SyncError::User("Google sign-in isn't set up (AXENTRA_GOOGLE_CLIENT_ID).".into()))?;
                if let Some(id) = cfg.google_client_id.as_deref().filter(|s| !s.is_empty()) {
                    c.client_id = id.to_string();
                }
                Ok(c)
            }
            _ => {
                let id = oauth::ms_client_id(cfg.ms_client_id.as_deref()).ok_or_else(|| SyncError::User(copy::NO_CLIENT_ID.into()))?;
                Ok(OAuthClient::microsoft(cfg.ms_tenant.as_deref().unwrap_or("common"), &id))
            }
        }
    }

    fn scopes_for(cfg: &AccountConfig, resource: Resource) -> &'static str {
        match (cfg.auth, resource) {
            (AuthMethod::Xoauth2Google, _) => "https://mail.google.com/",
            (_, Resource::Graph) => oauth::GRAPH_REFRESH_SCOPES,
            (_, Resource::Imap) => oauth::MS_IMAP_SCOPES,
            (_, Resource::Smtp) => oauth::MS_SMTP_SCOPES,
        }
    }

    /// Store a refresh token for an account (JSON per §7.2).
    pub async fn save_refresh_token(&self, account: &Account, cfg: &AccountConfig, token: &Secret, scopes: &str) -> SyncResult<()> {
        let value = json!({
            "refresh_token": token.expose(),
            "resource": if cfg.auth == AuthMethod::Xoauth2Google { "google" } else { "graph" },
            "tenant": cfg.ms_tenant.clone().unwrap_or_default(),
            "scopes": scopes,
        });
        self.keyring.set(&Self::secret_key(account, cfg, SecretKind::OauthRefreshToken), &Secret::new(value.to_string())).await
    }

    pub fn cache_token(&self, id: AccountId, resource: Resource, token: Secret, expires: DateTime<Utc>) {
        lock(&self.tokens).insert((id, resource), (token, expires));
    }

    /// A valid access token: cached, or refreshed (single-flight per account).
    pub async fn access_token(&self, account_id: AccountId, resource: Resource, force: bool) -> SyncResult<Secret> {
        let fresh = |inner: &Inner| {
            lock(&inner.tokens).get(&(account_id, resource)).filter(|(_, exp)| *exp - TOKEN_MARGIN > Utc::now()).map(|(t, _)| t.clone())
        };
        if !force && let Some(t) = fresh(self) {
            return Ok(t);
        }
        let gate = lock(&self.refresh_locks).entry(account_id).or_default().clone();
        let _guard = gate.lock().await;
        if !force && let Some(t) = fresh(self) {
            return Ok(t);
        }
        let account = self.store.account(account_id)?.ok_or(SyncError::Cancelled)?;
        let cfg = AccountConfig::of(&account);
        let key = Self::secret_key(&account, &cfg, SecretKind::OauthRefreshToken);
        let stored = self.keyring.get(&key, false).await?.ok_or_else(|| SyncError::Auth("Sign in again to keep this account up to date.".into()))?;
        let parsed: serde_json::Value = serde_json::from_str(stored.expose()).unwrap_or_default();
        let refresh = Secret::new(parsed["refresh_token"].as_str().unwrap_or(stored.expose()).to_string());
        let client = Self::oauth_client(&cfg)?;
        let scopes = Self::scopes_for(&cfg, resource);
        let tokens = oauth::refresh(&self.http, &client, &refresh, scopes).await?;
        if let Some(rt) = &tokens.refresh_token
            && rt != &refresh
        {
            // Replace the stored refresh token right away (§3.6).
            self.save_refresh_token(&account, &cfg, rt, parsed["scopes"].as_str().unwrap_or(scopes)).await?;
        }
        self.cache_token(account_id, resource, tokens.access_token.clone(), tokens.expires_at);
        Ok(tokens.access_token)
    }

    /// IMAP sign-in credential for an account.
    pub async fn imap_credential(&self, account: &Account, cfg: &AccountConfig) -> SyncResult<Credential> {
        match cfg.auth {
            AuthMethod::Password => {
                let pw = self
                    .keyring
                    .get(&Self::secret_key(account, cfg, SecretKind::Password), false)
                    .await?
                    .ok_or_else(|| SyncError::Auth("The saved password is missing. Sign in again.".into()))?;
                Ok(Credential::Password(pw))
            }
            _ => Ok(Credential::Bearer(self.access_token(account.id, Resource::Imap, false).await?)),
        }
    }

    pub async fn smtp_auth(&self, account: &Account, cfg: &AccountConfig) -> SyncResult<SmtpAuth> {
        let user = cfg.smtp_username(&account.email).to_string();
        match cfg.auth {
            AuthMethod::Password => {
                let kind = if cfg.smtp_same_auth { SecretKind::Password } else { SecretKind::SmtpPassword };
                let pw = self
                    .keyring
                    .get(&Self::secret_key(account, cfg, kind), false)
                    .await?
                    .ok_or_else(|| SyncError::Auth("The saved password is missing. Sign in again.".into()))?;
                Ok(SmtpAuth::Password { user, password: pw })
            }
            _ => Ok(SmtpAuth::Xoauth2 { user, token: self.access_token(account.id, Resource::Smtp, false).await? }),
        }
    }

    // ------------------------------------------------------------ scheduling

    /// Ask for a pass (merged with a running one).
    pub fn request(self: &Arc<Self>, id: AccountId, scope: Scope) {
        let start = {
            let mut sched = lock(&self.sched);
            let s = sched.entry(id).or_default();
            if s.running {
                s.queued = Some(s.queued.map_or(scope, |q| q.max(scope)));
                false
            } else {
                s.running = true;
                true
            }
        };
        if start {
            self.publish_status();
            let inner = self.clone();
            tokio::spawn(async move { inner.run(id, scope).await });
        }
    }

    pub fn request_all(self: &Arc<Self>, scope: Scope) {
        for a in self.store.accounts().unwrap_or_default() {
            if a.kind != AccountKind::Local {
                self.request(a.id, scope);
            }
        }
    }

    fn run(self: Arc<Self>, id: AccountId, scope: Scope) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let result = self.pass(id, scope).await;
            let worth_recording = match &result {
                Ok(()) => scope >= Scope::Quick,
                Err(SyncError::Cancelled) => false,
                Err(_) => true,
            };
            if worth_recording {
                self.record(id, &result);
            }
            if let Err(e) = &result {
                tracing::info!("sync of account {id} failed: {e}");
            }
            let next = {
                let mut sched = lock(&self.sched);
                let s = sched.entry(id).or_default();
                let now = Instant::now();
                match &result {
                    Ok(()) => {
                        s.failures = 0;
                        s.retry_at = None;
                        if scope >= Scope::Quick {
                            s.last_quick = Some(now);
                        }
                        if scope == Scope::Full {
                            s.last_full = Some(now);
                            s.first_pass_done = true;
                        }
                    }
                    Err(e) => {
                        s.failures += 1;
                        let wait = match e {
                            SyncError::Throttled { retry_after: Some(d) } => *d,
                            _ if e.is_transient() => Duration::from_secs((15u64 << s.failures.min(4)).min(300)),
                            _ => FULL_INTERVAL,
                        };
                        s.retry_at = Some(now + wait);
                        if scope >= Scope::Quick {
                            s.last_quick = Some(now);
                        }
                        if scope == Scope::Full {
                            s.last_full = Some(now);
                        }
                    }
                }
                s.progress = None;
                match s.queued.take() {
                    Some(q) if result.is_ok() || q > Scope::Outbox => Some(q),
                    _ => {
                        s.running = false;
                        None
                    }
                }
            };
            self.publish_status();
            if let Some(q) = next {
                self.clone().run(id, q).await;
            }
        })
    }

    /// Called every few seconds.
    pub fn tick(self: &Arc<Self>) {
        let accounts = self.store.accounts().unwrap_or_default();
        let ids: HashSet<AccountId> = accounts.iter().map(|a| a.id).collect();
        // Forget removed accounts (and stop their IDLE watchers).
        {
            let mut sched = lock(&self.sched);
            sched.retain(|id, s| {
                let keep = ids.contains(id);
                if !keep && let Some(h) = s.idle.take() {
                    h.abort();
                }
                keep
            });
        }
        let total = self.store.total_changes();
        let local_edits = {
            let mut last = lock(&self.last_total_changes);
            let changed = *last != total;
            *last = total;
            changed
        };
        let now = Instant::now();
        for a in accounts.iter().filter(|a| a.kind != AccountKind::Local) {
            let cfg = AccountConfig::of(a);
            let outbox_due = outbox::due(&self.store, a).map(|v| !v.is_empty()).unwrap_or(false);
            let scope = {
                let mut sched = lock(&self.sched);
                let s = sched.entry(a.id).or_default();
                if s.running {
                    continue;
                }
                if matches!(cfg.state, AccountState::NeedsReauth) {
                    continue;
                }
                if s.retry_at.is_some_and(|t| now < t) {
                    continue;
                }
                let quick_every = match a.kind {
                    AccountKind::Microsoft => GRAPH_QUICK,
                    _ if s.idle.as_ref().is_some_and(|h| !h.is_finished()) => IMAP_IDLE_QUICK,
                    _ => IMAP_POLL,
                };
                let full_due = s.last_full.is_none_or(|t| now - t >= FULL_INTERVAL);
                let quick_due = s.last_quick.is_none_or(|t| now - t >= quick_every);
                if full_due {
                    Some(Scope::Full)
                } else if quick_due {
                    Some(Scope::Quick)
                } else if outbox_due {
                    Some(Scope::Outbox)
                } else if local_edits && s.first_pass_done {
                    Some(Scope::Push)
                } else {
                    None
                }
            };
            if let Some(scope) = scope {
                self.request(a.id, scope);
            }
            if a.kind == AccountKind::Imap {
                self.ensure_idle(a, &cfg);
            }
        }
    }

    fn ensure_idle(self: &Arc<Self>, account: &Account, cfg: &AccountConfig) {
        let Some(server) = cfg.imap.clone() else { return };
        {
            let sched = lock(&self.sched);
            let Some(s) = sched.get(&account.id) else { return };
            if !s.first_pass_done || s.idle_unsupported || s.idle.as_ref().is_some_and(|h| !h.is_finished()) {
                return;
            }
            if !matches!(cfg.state, AccountState::Ok) {
                return;
            }
        }
        let inner = self.clone();
        let id = account.id;
        let user = cfg.imap_username(&account.email).to_string();
        let handle = tokio::spawn(async move {
            let creds = {
                let inner = inner.clone();
                move || {
                    let inner = inner.clone();
                    async move {
                        let account = inner.store.account(id)?.ok_or(SyncError::Cancelled)?;
                        let cfg = AccountConfig::of(&account);
                        inner.imap_credential(&account, &cfg).await
                    }
                }
            };
            let poke = {
                let inner = inner.clone();
                move || inner.request(id, Scope::Quick)
            };
            let end = imap::idle::watch(server, user, creds, poke).await;
            if matches!(end, imap::idle::IdleEnd::Unsupported)
                && let Some(s) = lock(&inner.sched).get_mut(&id)
            {
                s.idle_unsupported = true;
            }
        });
        if let Some(s) = lock(&self.sched).get_mut(&account.id) {
            s.idle = Some(handle);
        }
    }

    // ------------------------------------------------------------ passes

    async fn pass(self: &Arc<Self>, id: AccountId, scope: Scope) -> SyncResult<()> {
        let Some(account) = self.store.account(id)? else { return Err(SyncError::Cancelled) };
        let cfg = AccountConfig::of(&account);
        match account.kind {
            AccountKind::Local => Ok(()),
            AccountKind::Imap => self.imap_pass(&account, &cfg, scope).await,
            AccountKind::Microsoft => self.graph_pass(&account, &cfg, scope).await,
        }
    }

    async fn imap_pass(self: &Arc<Self>, account: &Account, cfg: &AccountConfig, scope: Scope) -> SyncResult<()> {
        let server = cfg.imap.clone().ok_or_else(|| SyncError::User("The incoming mail server isn't set up.".into()))?;
        let user = cfg.imap_username(&account.email).to_string();

        // Outbox first: sending is what the user is waiting for.
        let sent = self.imap_outbox(account, cfg).await?;
        if scope == Scope::Outbox && sent.is_empty() {
            return Ok(());
        }
        let mut ms = MailSync::new(&self.store, account, cfg, &self.notifier)?;
        if scope == Scope::Push && ms.pending_containers()?.is_empty() && !imap::folders::changes_pending(&self.store, account.id)? {
            return Ok(());
        }
        let credential = self.imap_credential(account, cfg).await?;
        let mut session = imap::open(&server, &user, &credential).await?;
        let caps: Vec<String> = {
            let mut c: Vec<String> = session.caps.iter().cloned().collect();
            c.sort();
            c
        };
        if caps != cfg.capabilities {
            let _ = self.store.update_account_with(account.id, |a| {
                let mut c = AccountConfig::of(a);
                c.capabilities = caps.clone();
                c.store_into(a);
            });
        }
        let mut remote = imap::folders::list(&mut session).await?;
        // Folders created / renamed / moved / deleted here go first, so the
        // mapping below (and moves into new folders) see the result.
        if imap::folders::push_changes(&mut session, &self.store, account.id, &remote).await? > 0 {
            remote = imap::folders::list(&mut session).await?;
        }
        let mapped = ms.map_folders(&remote)?;
        ms.delete_sent_drafts(&mut session).await?;

        // Sent copies for servers that don't file submissions themselves.
        for (message_id, bytes) in sent {
            self.sent_copy(&mut ms, &mut session, account, &message_id, &bytes).await;
        }

        match scope {
            Scope::Push | Scope::Outbox => {
                for container in ms.pending_containers()? {
                    ms.push_only(&mut session, &container).await?;
                }
            }
            Scope::Quick | Scope::Full => {
                let mut order = mapped.clone();
                order.sort_by_key(|m| (m.remote.role != Some(FolderRole::Inbox), m.remote.role.is_none()));
                let first = !lock(&self.sched).get(&account.id).is_some_and(|s| s.first_pass_done);
                let mut new_total = 0;
                let mut processed: HashSet<String> = HashSet::new();
                for m in &order {
                    if scope == Scope::Quick && m.remote.role != Some(FolderRole::Inbox) {
                        continue;
                    }
                    let report = ms.sync_folder(&mut session, m.folder_id, &m.remote.name).await?;
                    processed.insert(m.remote.name.clone());
                    new_total += report.new;
                    if first {
                        self.set_progress(account.id, Some(format!("Syncing your mailbox… {new_total} messages so far")));
                    }
                }
                for container in ms.pending_containers()? {
                    if !processed.contains(&container) {
                        ms.push_only(&mut session, &container).await?;
                    }
                }
            }
        }
        let _ = session.session.logout().await;
        Ok(())
    }

    /// Send due Outbox messages over SMTP. Returns (Message-ID, Sent copy)
    /// for messages whose Sent copy the app must append.
    async fn imap_outbox(&self, account: &Account, cfg: &AccountConfig) -> SyncResult<Vec<(String, Vec<u8>)>> {
        let due = outbox::due(&self.store, account)?;
        if due.is_empty() {
            return Ok(Vec::new());
        }
        let server = cfg.smtp.clone().ok_or_else(|| SyncError::User("The outgoing mail server isn't set up.".into()))?;
        let auth = self.smtp_auth(account, cfg).await?;
        let mut appends = Vec::new();
        let mut sent_count = 0;
        for m in due {
            if !self.outbox_ready(m.id) {
                continue;
            }
            let ready = match outbox::prepare(&self.store, account, &m) {
                Ok(r) => r,
                Err(e) => {
                    self.outbox_failed(account, &m, e)?;
                    continue;
                }
            };
            match smtp::send(&server, &auth, &ready.built.envelope_from, &ready.built.recipients, &ready.built.transmit).await {
                Ok(()) => {
                    outbox::finish(&self.store, account, m.id, &ready.message_id)?;
                    lock(&self.outbox_retry).remove(&m.id);
                    sent_count += 1;
                    let server_files = matches!(cfg.provider, Provider::Google | Provider::MicrosoftWork | Provider::MicrosoftPersonal)
                        || cfg.sent_copy_policy == SentCopyPolicy::Server;
                    if !server_files {
                        appends.push((ready.message_id.clone(), ready.built.sent_copy.clone()));
                    }
                    self.notifier.bump();
                }
                Err(e) => {
                    // Auth problems stop the whole pass; others are per message.
                    if matches!(e, SyncError::Auth(_)) {
                        return Err(e);
                    }
                    self.outbox_failed(account, &m, e)?;
                }
            }
        }
        if sent_count > 1 {
            self.notice(format!("{sent_count} messages sent"), false);
        }
        Ok(appends)
    }

    fn outbox_ready(&self, id: i64) -> bool {
        lock(&self.outbox_retry).get(&id).is_none_or(|r| Instant::now() >= r.next)
    }

    fn outbox_failed(&self, account: &Account, m: &axentra_core::Message, e: SyncError) -> SyncResult<()> {
        if e.is_transient() {
            let mut retry = lock(&self.outbox_retry);
            let r = retry.entry(m.id).or_insert(OutboxRetry { attempts: 0, next: Instant::now() });
            r.attempts += 1;
            let mins = [1u64, 5, 15, 60][(r.attempts as usize - 1).min(3)];
            r.next = Instant::now() + Duration::from_secs(mins * 60);
            return Ok(());
        }
        outbox::return_to_drafts(&self.store, account, m.id)?;
        lock(&self.outbox_retry).remove(&m.id);
        let subject = if m.subject.trim().is_empty() { "(No subject)" } else { m.subject.trim() };
        self.notice(format!("Couldn't send “{subject}”. Saved in Drafts. {e}"), true);
        self.notifier.bump();
        Ok(())
    }

    /// Apply the Sent-copy policy after an SMTP submission (§5.9).
    async fn sent_copy<T: imap::Transport>(&self, ms: &mut MailSync<'_>, session: &mut imap::Imap<T>, account: &Account, message_id: &str, bytes: &[u8]) {
        let Ok(Some(sent)) = self.store.folder_by_role(account.id, FolderRole::Sent) else { return };
        let cfg = AccountConfig::of(account);
        let policy = cfg.sent_copy_policy;
        let result: SyncResult<SentCopyPolicy> = async {
            match policy {
                SentCopyPolicy::ClientAppend => {
                    ms.append_sent(session, sent.id, bytes).await?;
                    Ok(SentCopyPolicy::ClientAppend)
                }
                SentCopyPolicy::Server => Ok(SentCopyPolicy::Server),
                SentCopyPolicy::Detect => {
                    // Give the server a moment to file its own copy.
                    tokio::time::sleep(Duration::from_secs(20)).await;
                    if ms.sent_has(session, sent.id, message_id).await? {
                        Ok(SentCopyPolicy::Server)
                    } else {
                        ms.append_sent(session, sent.id, bytes).await?;
                        Ok(SentCopyPolicy::ClientAppend)
                    }
                }
            }
        }
        .await;
        match result {
            Ok(p) if p != policy => {
                let _ = self.store.update_account_with(account.id, |a| {
                    let mut c = AccountConfig::of(a);
                    c.sent_copy_policy = p;
                    c.store_into(a);
                });
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("couldn't save the Sent copy: {e}"),
        }
    }

    fn graph_for(self: &Arc<Self>, account_id: AccountId) -> Graph {
        Graph::new(self.http.clone(), Arc::new(AccountTokens { inner: self.clone(), account_id }))
    }

    async fn graph_pass(self: &Arc<Self>, account: &Account, cfg: &AccountConfig, scope: Scope) -> SyncResult<()> {
        let graph = self.graph_for(account.id);
        self.graph_outbox(&graph, account).await?;
        if scope == Scope::Outbox {
            return Ok(());
        }
        let n = &self.notifier;
        let mut mail = GraphMail::new(&graph, &self.store, account, cfg, n)?;
        let cal = GraphCalendar { graph: &graph, store: &self.store, account, notifier: n };
        let todo = GraphTodo { graph: &graph, store: &self.store, account, notifier: n };
        match scope {
            Scope::Push | Scope::Outbox => {
                mail.push_folders().await?;
                mail.delete_sent_drafts().await?;
                mail.push().await?;
                cal.push_only().await?;
                todo.push_only().await?;
                contacts::push(&graph, &self.store, account, n).await?;
            }
            Scope::Quick => {
                if mail.folders_pending()? {
                    mail.push_folders().await?;
                }
                if mail.remote_of.is_empty() {
                    mail.sync_folders().await?;
                }
                mail.delete_sent_drafts().await?;
                mail.sync_messages(Some(&[FolderRole::Inbox])).await?;
                mail.push().await?;
                cal.push_only().await?;
                todo.push_only().await?;
            }
            Scope::Full => {
                let first = !lock(&self.sched).get(&account.id).is_some_and(|s| s.first_pass_done);
                if first {
                    self.set_progress(account.id, Some("Syncing your mailbox… 0 messages so far".into()));
                }
                mail.push_folders().await?;
                mail.sync_folders().await?;
                mail.delete_sent_drafts().await?;
                let inbox = mail.sync_messages(Some(&[FolderRole::Inbox])).await?;
                if first {
                    self.set_progress(account.id, Some(format!("Syncing your mailbox… {inbox} messages so far")));
                }
                for (cid, rid) in cal.sync_calendars().await? {
                    cal.sync_calendar(cid, &rid).await?;
                }
                let others = [
                    FolderRole::Drafts,
                    FolderRole::Sent,
                    FolderRole::Deleted,
                    FolderRole::Junk,
                    FolderRole::Archive,
                    FolderRole::ConversationHistory,
                    FolderRole::Custom,
                ];
                let rest = mail.sync_messages(Some(&others)).await?;
                if first {
                    self.set_progress(account.id, Some(format!("Syncing your mailbox… {} messages so far", inbox + rest)));
                }
                mail.push().await?;
                todo.sync().await?;
                let contacts_due = lock(&self.sched).get(&account.id).is_none_or(|s| s.last_contacts.is_none_or(|t| t.elapsed() >= CONTACTS_INTERVAL));
                if contacts_due {
                    contacts::sync(&graph, &self.store, account, n).await?;
                    if let Some(s) = lock(&self.sched).get_mut(&account.id) {
                        s.last_contacts = Some(Instant::now());
                    }
                } else {
                    contacts::push(&graph, &self.store, account, n).await?;
                }
            }
        }
        Ok(())
    }

    /// Send due Outbox messages with `POST /me/sendMail` (MIME, base64).
    /// Graph saves the copy to Sent Items itself.
    async fn graph_outbox(&self, graph: &Graph, account: &Account) -> SyncResult<()> {
        use base64::Engine as _;
        let mut sent_count = 0;
        for m in outbox::due(&self.store, account)? {
            if !self.outbox_ready(m.id) {
                continue;
            }
            let ready = match outbox::prepare(&self.store, account, &m) {
                Ok(r) => r,
                Err(e) => {
                    self.outbox_failed(account, &m, e)?;
                    continue;
                }
            };
            // sendMail with MIME is limited to about 4 MB per request.
            if ready.built.transmit.len() > 3_500_000 {
                self.outbox_failed(account, &m, SyncError::User(copy::TOO_LARGE.into()))?;
                continue;
            }
            let b64 = base64::engine::general_purpose::STANDARD.encode(&ready.built.transmit);
            match graph.post_text("/me/sendMail", "text/plain", b64).await {
                Ok(()) => {
                    outbox::finish(&self.store, account, m.id, &ready.message_id)?;
                    lock(&self.outbox_retry).remove(&m.id);
                    sent_count += 1;
                    self.notifier.bump();
                }
                Err(e @ (SyncError::Auth(_) | SyncError::Keyring(_))) => return Err(e),
                Err(e) => self.outbox_failed(account, &m, e)?,
            }
        }
        if sent_count > 1 {
            self.notice(format!("{sent_count} messages sent"), false);
        }
        Ok(())
    }

    // ------------------------------------------------------------ account removal

    pub async fn forget_account(&self, account: &Account) -> SyncResult<()> {
        let cfg = AccountConfig::of(account);
        if let Some(s) = lock(&self.sched).get_mut(&account.id)
            && let Some(h) = s.idle.take()
        {
            h.abort();
        }
        lock(&self.tokens).retain(|(id, _), _| *id != account.id);
        if !cfg.uuid.is_empty() {
            // Google asks clients to revoke on removal (§3.7); best effort.
            if cfg.auth == AuthMethod::Xoauth2Google
                && let Ok(Some(stored)) = self.keyring.get(&Self::secret_key(account, &cfg, SecretKind::OauthRefreshToken), false).await
            {
                let v: serde_json::Value = serde_json::from_str(stored.expose()).unwrap_or_default();
                if let Some(rt) = v["refresh_token"].as_str() {
                    let _ = self.http.post("https://oauth2.googleapis.com/revoke").form(&[("token", rt)]).send().await;
                }
            }
            match self.keyring.delete_account(&cfg.uuid).await {
                Ok(()) | Err(SyncError::Keyring(KeyringError::Unavailable)) => {}
                Err(e) => tracing::warn!("couldn't delete keyring items: {e}"),
            }
        }
        self.store.remove_account(account.id)?;
        lock(&self.sched).remove(&account.id);
        self.notifier.bump();
        self.publish_status();
        Ok(())
    }
}

/// Graph's token source for one account.
struct AccountTokens {
    inner: Arc<Inner>,
    account_id: AccountId,
}

impl TokenSource for AccountTokens {
    fn token(&self, force: bool) -> BoxFuture<'_, SyncResult<Secret>> {
        Box::pin(self.inner.access_token(self.account_id, Resource::Graph, force))
    }
}
