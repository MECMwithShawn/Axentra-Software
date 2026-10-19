//! Account connectivity: Microsoft Graph and IMAP/SMTP sync into the local
//! store. Runs on its own thread with a Tokio runtime; the UI talks to it
//! through a cloneable [`SyncHandle`].
//!
//! See `docs/spec/protocols.md` (behaviour) and `docs/ACCOUNTS.md` (setting
//! up a Microsoft Entra app registration).

pub mod accounts;
pub mod config;
pub mod diff;
pub mod discover;
mod engine;
pub mod error;
pub mod graph;
pub mod imap;
pub mod mime;
pub mod notify;
pub mod oauth;
pub mod outbox;
pub mod secrets;
pub mod smtp;
mod tls;

use std::future::Future;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use axentra_core::{AccountId, Store};
use tokio::sync::mpsc;

pub use accounts::{DeviceCodeInfo, ImapSetup, SignInKind, SignInRequest, SignInTask, SignedIn};
pub use config::{AccountConfig, AccountHealth, AccountState, Provider, Security, ServerConfig};
pub use discover::Detection;
pub use error::{SyncError, SyncResult};
pub use secrets::{Keyring, Secret};

use engine::{Inner, Scope};
use notify::Notifier;

/// What the UI can ask the engine to do.
#[derive(Debug)]
pub enum Command {
    /// Sync every account now.
    SyncAll,
    /// Sync one account now.
    SyncAccount(AccountId),
    /// Send everything waiting in Outbox folders.
    FlushOutbox,
    Shutdown,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SyncStatus {
    pub running: bool,
    /// Human-readable per-account state, e.g. "Updated just now".
    pub accounts: Vec<(AccountId, String)>,
    pub last_error: Option<String>,
    /// Live detail per account (same order as `accounts`).
    pub details: Vec<AccountSyncInfo>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountSyncInfo {
    pub account_id: AccountId,
    pub syncing: bool,
    /// The very first sync of a new account is running.
    pub first_sync: bool,
    /// "Syncing your mailbox… 120 messages so far".
    pub progress: Option<String>,
    pub status_line: String,
    pub error: Option<String>,
}

/// A message for an in-app toast ("Couldn't send …", "3 messages sent").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    /// Show as an error toast.
    pub error: bool,
}

#[derive(Clone)]
pub struct SyncHandle {
    tx: mpsc::UnboundedSender<Command>,
    changes: Arc<AtomicU64>,
    status: Arc<Mutex<SyncStatus>>,
    inner: Arc<Inner>,
    rt: tokio::runtime::Handle,
}

impl SyncHandle {
    /// Start the engine on a background thread, with secrets in the
    /// freedesktop Secret Service.
    pub fn start(store: Arc<Store>) -> SyncHandle {
        Self::start_with_keyring(store, Keyring::SecretService)
    }

    /// Start with a specific keyring (tests use [`Keyring::memory`]).
    pub fn start_with_keyring(store: Arc<Store>, keyring: Keyring) -> SyncHandle {
        let (tx, mut rx) = mpsc::unbounded_channel::<Command>();
        let changes = Arc::new(AtomicU64::new(0));
        let status = Arc::new(Mutex::new(SyncStatus::default()));
        let inner = Inner::new(store, Notifier::new(changes.clone()), status.clone(), keyring);
        let (rt_tx, rt_rx) = std::sync::mpsc::channel();
        let engine = inner.clone();
        std::thread::Builder::new()
            .name("axentra-sync".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .thread_name("axentra-sync-worker")
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                let _ = rt_tx.send(rt.handle().clone());
                rt.block_on(async move {
                    engine.reset_transient_states();
                    // Let the window come up before the first network burst.
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    let mut tick = tokio::time::interval(engine::TICK);
                    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        tokio::select! {
                            cmd = rx.recv() => match cmd {
                                None | Some(Command::Shutdown) => break,
                                Some(Command::SyncAll) => engine.request_all(Scope::Full),
                                Some(Command::SyncAccount(id)) => engine.request(id, Scope::Full),
                                Some(Command::FlushOutbox) => engine.request_all(Scope::Outbox),
                            },
                            _ = tick.tick() => engine.tick(),
                        }
                    }
                });
            })
            .expect("spawn sync thread");
        let rt = rt_rx.recv().expect("sync runtime started");
        SyncHandle { tx, changes, status, inner, rt }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    pub fn sync_now(&self) {
        self.send(Command::SyncAll);
    }

    /// Sync one account now (the "Sync now" / "Retry" buttons).
    pub fn sync_account(&self, id: AccountId) {
        self.send(Command::SyncAccount(id));
    }

    /// Increases whenever the engine wrote to the store; the UI polls it.
    pub fn change_counter(&self) -> u64 {
        self.changes.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn status(&self) -> SyncStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Live detail for one account.
    pub fn account_status(&self, id: AccountId) -> Option<AccountSyncInfo> {
        self.status().details.into_iter().find(|d| d.account_id == id)
    }

    /// Messages for in-app toasts ("Couldn't send …", "3 messages sent"),
    /// each returned once. The shell should drain this with the change
    /// counter and show `app.error(..)` / `app.toast(..)`.
    pub fn take_notices(&self) -> Vec<Notice> {
        self.inner.take_notices()
    }

    /// Run a future on the engine's runtime (network code needs it).
    fn run<T, F>(&self, fut: F) -> impl Future<Output = SyncResult<T>> + use<T, F>
    where
        T: Send + 'static,
        F: Future<Output = SyncResult<T>> + Send + 'static,
    {
        let handle = self.rt.spawn(fut);
        async move { handle.await.unwrap_or(Err(SyncError::Cancelled)) }
    }

    /// "Looking up your account…" (§1.3).
    pub fn detect(&self, email: String) -> impl Future<Output = SyncResult<Detection>> + use<> {
        let inner = self.inner.clone();
        self.run(async move { Ok(discover::detect(&inner.http, &email).await) })
    }

    /// Sign in to IMAP and SMTP without saving ("Test connection").
    pub fn test_imap(&self, setup: ImapSetup) -> impl Future<Output = SyncResult<Vec<String>>> + use<> {
        self.run(async move { accounts::test_imap(&setup).await })
    }

    /// Test, save the password in the keyring and add the account.
    pub fn add_imap(&self, setup: ImapSetup) -> impl Future<Output = SyncResult<AccountId>> + use<> {
        let inner = self.inner.clone();
        self.run(async move { accounts::add_imap(&inner, setup).await })
    }

    /// Store a new password for an IMAP account that needs to sign in again.
    pub fn update_password(&self, id: AccountId, password: Secret) -> impl Future<Output = SyncResult<()>> + use<> {
        let inner = self.inner.clone();
        self.run(async move { accounts::update_password(&inner, id, password).await })
    }

    /// Open the browser for OAuth sign-in. Returns the authorize URL and
    /// the task to await (abort it on Cancel).
    pub fn start_browser_sign_in(&self, req: SignInRequest) -> impl Future<Output = SyncResult<(String, SignInTask)>> + use<> {
        let inner = self.inner.clone();
        self.run(async move { accounts::start_browser(&inner, req).await })
    }

    /// Device code sign-in ("Use a code instead").
    pub fn start_device_sign_in(&self, req: SignInRequest) -> impl Future<Output = SyncResult<(DeviceCodeInfo, SignInTask)>> + use<> {
        let inner = self.inner.clone();
        self.run(async move { accounts::start_device(&inner, req).await })
    }

    /// Create the account from a finished sign-in.
    pub fn finish_sign_in(&self, flow: u64) -> impl Future<Output = SyncResult<AccountId>> + use<> {
        let inner = self.inner.clone();
        self.run(async move { accounts::finish(&inner, flow).await })
    }

    /// Remove an account with its keyring items and local data.
    pub fn remove_account(&self, id: AccountId) -> impl Future<Output = SyncResult<()>> + use<> {
        let inner = self.inner.clone();
        self.run(async move { accounts::remove(&inner, id).await })
    }

    /// Unlock the keyring for an account (prompts) and sync it.
    pub fn unlock_keyring(&self, id: AccountId) -> impl Future<Output = SyncResult<()>> + use<> {
        let inner = self.inner.clone();
        self.run(async move { accounts::unlock_keyring(&inner, id).await })
    }

    /// Open a link in the system browser.
    pub fn open_url(&self, url: &str) -> SyncResult<()> {
        oauth::open_in_browser(url)
    }
}

#[cfg(test)]
mod tests;
