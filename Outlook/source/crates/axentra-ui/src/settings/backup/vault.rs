//! The backup password in the keyring, through the sync engine's keyring
//! (freedesktop Secret Service). The preview server has no engine; it can
//! opt into an in-memory keyring instead ([`use_memory_keyring`]).

use std::sync::OnceLock;

use axentra_sync::error::KeyringError;
use axentra_sync::secrets::backup_password_key;
use axentra_sync::{Keyring, Secret, SyncError};

use crate::state::sync;

static MEMORY: OnceLock<Keyring> = OnceLock::new();

/// Keep the backup password in memory for this process (preview server
/// and tests; never the desktop app).
pub fn use_memory_keyring() {
    let _ = MEMORY.set(Keyring::memory());
}

/// What the keyring can do for backups right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyringState {
    /// A backup password is remembered.
    Remembered,
    /// The keyring works but holds no backup password.
    NotSet,
    /// The keyring is locked; unlocking asks the user.
    Locked,
    /// No keyring: passwords can't be remembered.
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultError {
    Unavailable,
    Locked,
    Denied,
    Other(String),
}

impl VaultError {
    pub fn message(&self) -> String {
        match self {
            VaultError::Unavailable => {
                "No keyring is available to remember the backup password. You can still back up by typing it each time.".into()
            }
            VaultError::Locked => "Your keyring is locked.".into(),
            VaultError::Denied => "Access to the keyring was denied.".into(),
            VaultError::Other(m) => m.clone(),
        }
    }
}

fn map(e: SyncError) -> VaultError {
    match e {
        SyncError::Keyring(KeyringError::Unavailable) => VaultError::Unavailable,
        SyncError::Keyring(KeyringError::Locked) => VaultError::Locked,
        SyncError::Keyring(KeyringError::Denied) => VaultError::Denied,
        other => VaultError::Other(other.to_string()),
    }
}

/// The remembered password. `allow_prompt` lets a locked keyring ask to be
/// unlocked (only for actions the user started).
pub async fn get(allow_prompt: bool) -> Result<Option<Secret>, VaultError> {
    if let Some(k) = MEMORY.get() {
        return k.get(&backup_password_key(), allow_prompt).await.map_err(map);
    }
    match sync() {
        Some(s) => s.backup_password(allow_prompt).await.map_err(map),
        None => Err(VaultError::Unavailable),
    }
}

pub async fn set(password: Secret) -> Result<(), VaultError> {
    if let Some(k) = MEMORY.get() {
        return k.set(&backup_password_key(), &password).await.map_err(map);
    }
    match sync() {
        Some(s) => s.set_backup_password(password).await.map_err(map),
        None => Err(VaultError::Unavailable),
    }
}

pub async fn forget() -> Result<(), VaultError> {
    if let Some(k) = MEMORY.get() {
        return k.delete(&backup_password_key()).await.map_err(map);
    }
    match sync() {
        Some(s) => s.forget_backup_password().await.map_err(map),
        None => Err(VaultError::Unavailable),
    }
}

/// Probe the keyring without prompting.
pub async fn state() -> KeyringState {
    match get(false).await {
        Ok(Some(_)) => KeyringState::Remembered,
        Ok(None) => KeyringState::NotSet,
        Err(VaultError::Locked) => KeyringState::Locked,
        Err(_) => KeyringState::Unavailable,
    }
}
