//! Credential storage in the freedesktop Secret Service (GNOME Keyring,
//! KWallet, KeePassXC) over D-Bus, per docs/spec/protocols.md §7. Nothing
//! secret is ever written to SQLite, config files or logs; when no keyring is
//! available the caller gets [`KeyringError::Unavailable`] and must tell the
//! user instead of falling back to plain text.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use secret_service::{EncryptionType, SecretService};

use crate::config::Provider;
use crate::error::{KeyringError, SyncError, SyncResult};

const SCHEMA: &str = "app.axentra.Outlook.Credential";
const CONTENT_TYPE: &str = "text/plain; charset=utf8";

/// A secret string that never prints and is overwritten when dropped.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Secret(s.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.fill(0);
        // Keep the compiler from eliding the wipe of a buffer about to be freed.
        std::hint::black_box(&bytes);
    }
}

/// Kinds of credential (the keyring `kind` attribute).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SecretKind {
    /// JSON `{"refresh_token","resource","tenant","scopes"}` (§7.2).
    OauthRefreshToken,
    /// IMAP password / app password.
    Password,
    /// SMTP password when SMTP signs in separately.
    SmtpPassword,
    /// The password that encrypts account backups (one per user, see
    /// [`backup_password_key`]); remembered so monthly backups run unattended.
    BackupPassword,
}

impl SecretKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SecretKind::OauthRefreshToken => "oauth_refresh_token",
            SecretKind::Password => "password",
            SecretKind::SmtpPassword => "smtp_password",
            SecretKind::BackupPassword => "backup_password",
        }
    }
}

/// Keyring `account_id` attribute of the backup password: not an account,
/// so removing an account never deletes it.
pub const BACKUP_SECRET_ID: &str = "axentra-outlook-backup";

/// Where the backup password is kept.
pub fn backup_password_key() -> SecretKey {
    SecretKey {
        account_uuid: BACKUP_SECRET_ID.into(),
        kind: SecretKind::BackupPassword,
        provider: Provider::Imap,
        username: "backup".into(),
    }
}

/// Identifies one stored credential.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SecretKey {
    /// `AccountConfig::uuid`.
    pub account_uuid: String,
    pub kind: SecretKind,
    pub provider: Provider,
    pub username: String,
}

impl SecretKey {
    fn attributes(&self) -> HashMap<&str, &str> {
        HashMap::from([
            ("xdg:schema", SCHEMA),
            ("account_id", self.account_uuid.as_str()),
            ("kind", self.kind.as_str()),
            ("provider", if self.kind == SecretKind::BackupPassword { "backup" } else { self.provider.keyring_name() }),
            ("username", self.username.as_str()),
        ])
    }

    /// Lookup attributes: everything but the user name (which may change
    /// when an address is renamed).
    fn lookup(&self) -> HashMap<&str, &str> {
        HashMap::from([("xdg:schema", SCHEMA), ("account_id", self.account_uuid.as_str()), ("kind", self.kind.as_str())])
    }

    fn label(&self) -> String {
        if self.kind == SecretKind::BackupPassword {
            return "Axentra Outlook – backup password".into();
        }
        format!("Axentra Outlook – {} ({})", self.username, self.provider.keyring_name())
    }
}

/// Where secrets go. `Memory` is used by tests and never by the app.
pub enum Keyring {
    SecretService,
    Memory(Mutex<HashMap<(String, SecretKind), Secret>>),
}

impl Keyring {
    pub fn memory() -> Self {
        Keyring::Memory(Mutex::new(HashMap::new()))
    }

    /// Read a secret. With `allow_prompt` a locked keyring asks the user to
    /// unlock it; without, a locked keyring returns `Locked`.
    pub async fn get(&self, key: &SecretKey, allow_prompt: bool) -> SyncResult<Option<Secret>> {
        match self {
            Keyring::Memory(m) => Ok(m.lock().unwrap_or_else(|p| p.into_inner()).get(&(key.account_uuid.clone(), key.kind)).cloned()),
            Keyring::SecretService => {
                let ss = connect().await?;
                let found = ss.search_items(key.lookup()).await.map_err(map_err)?;
                if let Some(item) = found.unlocked.first() {
                    let bytes = item.get_secret().await.map_err(map_err)?;
                    return Ok(Some(Secret::new(String::from_utf8_lossy(&bytes).into_owned())));
                }
                let Some(item) = found.locked.first() else { return Ok(None) };
                if !allow_prompt {
                    return Err(SyncError::Keyring(KeyringError::Locked));
                }
                item.unlock().await.map_err(map_err)?;
                let bytes = item.get_secret().await.map_err(map_err)?;
                Ok(Some(Secret::new(String::from_utf8_lossy(&bytes).into_owned())))
            }
        }
    }

    /// Create or replace a secret (atomic in the Secret Service: `replace=true`).
    pub async fn set(&self, key: &SecretKey, value: &Secret) -> SyncResult<()> {
        match self {
            Keyring::Memory(m) => {
                m.lock().unwrap_or_else(|p| p.into_inner()).insert((key.account_uuid.clone(), key.kind), value.clone());
                Ok(())
            }
            Keyring::SecretService => {
                let ss = connect().await?;
                let collection = ss.get_default_collection().await.map_err(map_err)?;
                if collection.is_locked().await.map_err(map_err)? {
                    collection.unlock().await.map_err(map_err)?;
                }
                collection
                    .create_item(&key.label(), key.attributes(), value.expose().as_bytes(), true, CONTENT_TYPE)
                    .await
                    .map_err(map_err)?;
                Ok(())
            }
        }
    }

    /// Delete one secret (no error when it doesn't exist).
    pub async fn delete(&self, key: &SecretKey) -> SyncResult<()> {
        match self {
            Keyring::Memory(m) => {
                m.lock().unwrap_or_else(|p| p.into_inner()).remove(&(key.account_uuid.clone(), key.kind));
                Ok(())
            }
            Keyring::SecretService => {
                let ss = connect().await?;
                let found = ss.search_items(key.lookup()).await.map_err(map_err)?;
                for item in found.unlocked.iter().chain(found.locked.iter()) {
                    item.delete().await.map_err(map_err)?;
                }
                Ok(())
            }
        }
    }

    /// Delete every secret of an account (on account removal).
    pub async fn delete_account(&self, account_uuid: &str) -> SyncResult<()> {
        match self {
            Keyring::Memory(m) => {
                m.lock().unwrap_or_else(|p| p.into_inner()).retain(|(uuid, _), _| uuid != account_uuid);
                Ok(())
            }
            Keyring::SecretService => {
                let ss = connect().await?;
                let attrs = HashMap::from([("xdg:schema", SCHEMA), ("account_id", account_uuid)]);
                let found = ss.search_items(attrs).await.map_err(map_err)?;
                for item in found.unlocked.iter().chain(found.locked.iter()) {
                    item.delete().await.map_err(map_err)?;
                }
                Ok(())
            }
        }
    }

    /// Unlock the collection holding an account's secrets (the "Unlock"
    /// button for the `keyring_locked` state).
    pub async fn unlock(&self, account_uuid: &str) -> SyncResult<()> {
        match self {
            Keyring::Memory(_) => Ok(()),
            Keyring::SecretService => {
                let ss = connect().await?;
                let attrs = HashMap::from([("xdg:schema", SCHEMA), ("account_id", account_uuid)]);
                let found = ss.search_items(attrs).await.map_err(map_err)?;
                let locked: Vec<_> = found.locked.iter().collect();
                if !locked.is_empty() {
                    ss.unlock_all(&locked).await.map_err(map_err)?;
                }
                Ok(())
            }
        }
    }
}

async fn connect() -> SyncResult<SecretService<'static>> {
    match SecretService::connect(EncryptionType::Dh).await {
        Ok(ss) => Ok(ss),
        // Some providers only implement the plain algorithm (allowed: the
        // session bus is per-user, §7.2).
        Err(secret_service::Error::Crypto(_)) => SecretService::connect(EncryptionType::Plain).await.map_err(map_err),
        Err(e) => Err(map_err(e)),
    }
}

fn map_err(e: secret_service::Error) -> SyncError {
    use secret_service::Error as E;
    match e {
        E::Locked => SyncError::Keyring(KeyringError::Locked),
        E::Prompt => SyncError::Keyring(KeyringError::Denied),
        E::Unavailable => SyncError::Keyring(KeyringError::Unavailable),
        E::Zbus(z) => {
            tracing::warn!("secret service: {z}");
            SyncError::Keyring(KeyringError::Unavailable)
        }
        E::ZbusFdo(z) => {
            tracing::warn!("secret service: {z}");
            SyncError::Keyring(KeyringError::Unavailable)
        }
        other => {
            tracing::warn!("secret service: {other}");
            SyncError::Keyring(KeyringError::Unavailable)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(kind: SecretKind) -> SecretKey {
        SecretKey { account_uuid: "u1".into(), kind, provider: Provider::Imap, username: "ana@example.org".into() }
    }

    #[test]
    fn secret_is_redacted_and_memory_store_works() {
        let s = Secret::new("hunter2");
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let k = Keyring::memory();
            assert!(k.get(&key(SecretKind::Password), false).await.unwrap().is_none());
            k.set(&key(SecretKind::Password), &s).await.unwrap();
            k.set(&key(SecretKind::SmtpPassword), &Secret::new("x")).await.unwrap();
            assert_eq!(k.get(&key(SecretKind::Password), false).await.unwrap().unwrap().expose(), "hunter2");
            k.delete_account("u1").await.unwrap();
            assert!(k.get(&key(SecretKind::SmtpPassword), false).await.unwrap().is_none());
        });
        assert_eq!(key(SecretKind::OauthRefreshToken).label(), "Axentra Outlook – ana@example.org (imap)");
        assert_eq!(key(SecretKind::Password).attributes()["xdg:schema"], SCHEMA);
    }

    #[test]
    fn backup_password_is_its_own_item_and_survives_account_removal() {
        let b = backup_password_key();
        assert_eq!(b.attributes()["kind"], "backup_password");
        assert_eq!(b.attributes()["provider"], "backup");
        assert_eq!(b.attributes()["account_id"], BACKUP_SECRET_ID);
        assert_eq!(b.label(), "Axentra Outlook – backup password");
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let k = Keyring::memory();
            k.set(&b, &Secret::new("backup pass")).await.unwrap();
            k.set(&key(SecretKind::Password), &Secret::new("imap pass")).await.unwrap();
            k.delete_account("u1").await.unwrap();
            assert_eq!(k.get(&b, false).await.unwrap().unwrap().expose(), "backup pass");
            k.delete(&b).await.unwrap();
            assert!(k.get(&b, false).await.unwrap().is_none());
            // Deleting what isn't there is fine.
            k.delete(&b).await.unwrap();
        });
    }
}
