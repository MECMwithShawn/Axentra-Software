//! The Microsoft application (client) ID that sign-in uses. Axentra ships
//! no ID (users register their own Entra app, docs/ACCOUNTS.md), so the last
//! ID that signed in successfully is remembered and offered next time:
//!
//! * in `$XDG_CONFIG_HOME/axentra-outlook/microsoft-client-id`, shared by
//!   every profile (database, `AXENTRA_DB`) of this user;
//! * in the profile's own key/value table (`ms_client_id`).
//!
//! An ID only names an app registration; it is not a secret.
//!
//! Lookup order: the ID typed in the dialog (or stored on the account), then
//! `AXENTRA_MS_CLIENT_ID`, then the remembered one.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use axentra_core::{AccountKind, Store};

use crate::config::AccountConfig;

/// Environment variable that overrides the remembered ID.
pub const ENV: &str = "AXENTRA_MS_CLIENT_ID";
/// Key in the store's key/value table.
pub const KV_KEY: &str = "ms_client_id";
const FILE_NAME: &str = "microsoft-client-id";

/// `None` = not looked up yet; `Some(None)` = looked up, nothing remembered.
static CACHE: Mutex<Option<Option<String>>> = Mutex::new(None);

/// A usable client ID: trimmed, non-empty, one token of at most 128
/// characters (Entra IDs are GUIDs; anything else is left to the server).
pub fn clean(id: &str) -> Option<String> {
    let id = id.trim();
    let ok = !id.is_empty() && id.len() <= 128 && !id.chars().any(|c| c.is_whitespace() || c.is_control());
    ok.then(|| id.to_string())
}

/// First usable ID of: the dialog / account value, the environment
/// variable, the remembered ID.
pub fn resolve(configured: Option<&str>, env: Option<&str>, remembered: Option<&str>) -> Option<String> {
    [configured, env, remembered].into_iter().flatten().find_map(clean)
}

/// Microsoft client ID for a sign-in or refresh (see the module docs).
pub fn ms_client_id(configured: Option<&str>) -> Option<String> {
    let env = std::env::var(ENV).ok();
    resolve(configured, env.as_deref(), remembered().as_deref())
}

/// The remembered ID (read from the shared file once per process).
pub fn remembered() -> Option<String> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.get_or_insert_with(|| config_file().as_deref().and_then(read_file)).clone()
}

/// Remember the ID a sign-in just succeeded with.
pub fn remember(store: &Store, id: &str) {
    let Some(id) = clean(id) else { return };
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Some(id.clone()));
    persist(store, &id, config_file().as_deref());
}

/// At startup: a profile that signed in before IDs were remembered (0.1.0)
/// still has its ID on the account; remember it so new profiles get it too.
pub fn seed_from_store(store: &Store) {
    if remembered().is_none()
        && let Some(id) = stored_id(store)
    {
        remember(store, &id);
    }
}

/// The ID this profile knows: its key/value entry, else the first
/// Microsoft account's.
fn stored_id(store: &Store) -> Option<String> {
    store.kv_get(KV_KEY).ok().flatten().and_then(|v| clean(&v)).or_else(|| {
        store
            .accounts()
            .ok()?
            .iter()
            .filter(|a| a.kind == AccountKind::Microsoft)
            .find_map(|a| AccountConfig::of(a).ms_client_id.as_deref().and_then(clean))
    })
}

fn persist(store: &Store, id: &str, file: Option<&Path>) {
    if let Err(e) = store.kv_set(KV_KEY, id) {
        tracing::warn!("remember client id in the store: {e}");
    }
    if let Some(path) = file
        && let Err(e) = write_file(path, id)
    {
        tracing::warn!("remember client id in {}: {e}", path.display());
    }
}

/// `$XDG_CONFIG_HOME/axentra-outlook/microsoft-client-id`. Unit tests never
/// read or write the real file.
fn config_file() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("axentra-outlook").join(FILE_NAME))
}

fn read_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().and_then(|s| clean(&s))
}

fn write_file(path: &Path, id: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write next to it and rename, so a crash never leaves half an ID.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, format!("{id}\n"))?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axentra_core::Account;

    const DIALOG: &str = "11111111-1111-1111-1111-111111111111";
    const ENV_ID: &str = "22222222-2222-2222-2222-222222222222";
    const KEPT: &str = "33333333-3333-3333-3333-333333333333";

    #[test]
    fn dialog_beats_environment_beats_remembered() {
        assert_eq!(resolve(Some(DIALOG), Some(ENV_ID), Some(KEPT)).as_deref(), Some(DIALOG));
        assert_eq!(resolve(None, Some(ENV_ID), Some(KEPT)).as_deref(), Some(ENV_ID));
        assert_eq!(resolve(None, None, Some(KEPT)).as_deref(), Some(KEPT));
        // Blank or broken values fall through to the next source.
        assert_eq!(resolve(Some("   "), Some(""), Some(KEPT)).as_deref(), Some(KEPT));
        assert_eq!(resolve(Some(" 1111 2222 "), Some(&format!(" {ENV_ID}\n")), None).as_deref(), Some(ENV_ID));
        assert_eq!(resolve(None, None, None), None);
    }

    #[test]
    fn the_file_round_trips_and_ignores_junk() {
        let dir = std::env::temp_dir().join(format!("axentra-client-id-{}", uuid::Uuid::new_v4()));
        let path = dir.join("nested").join(FILE_NAME);
        assert_eq!(read_file(&path), None);
        write_file(&path, KEPT).unwrap();
        assert_eq!(read_file(&path).as_deref(), Some(KEPT));
        std::fs::write(&path, "\n \n").unwrap();
        assert_eq!(read_file(&path), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_profile_knows_its_id_from_the_store_or_its_microsoft_account() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(stored_id(&store), None);
        let cfg = AccountConfig { ms_client_id: Some(format!(" {DIALOG} ")), ..Default::default() };
        let mut account = Account { kind: AccountKind::Microsoft, email: "ana@contoso.example".into(), ..Default::default() };
        cfg.store_into(&mut account);
        store.add_account(&mut account).unwrap();
        assert_eq!(stored_id(&store).as_deref(), Some(DIALOG), "0.1.0 profiles: the account's ID");
        let dir = std::env::temp_dir().join(format!("axentra-client-id-{}", uuid::Uuid::new_v4()));
        let file = dir.join(FILE_NAME);
        persist(&store, KEPT, Some(&file));
        assert_eq!(stored_id(&store).as_deref(), Some(KEPT), "the remembered entry wins over the account");
        assert_eq!(read_file(&file).as_deref(), Some(KEPT));
        let _ = std::fs::remove_dir_all(dir);
    }
}
