//! Slow work (Argon2 key derivation, passkey dialogs) runs on a worker thread so the window stays
//! responsive; results come back over a channel polled each frame.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use passkey_win::{PasskeyError, PrfCredential, Registration};
use vault_core::backups::BackupConfig;
use vault_core::envelope::{self, Envelope, KeyRing, RingKey};
use vault_core::header::{Kdf, KdfBudget};
use vault_core::vault::{Credentials, SealedVault, Vault, VaultError};
use vault_core::{limits, random_bytes};
use zeroize::Zeroizing;

/// Relying-party ID for passkeys, shared with the KeePass plugin (proven with iCloud Keychain).
pub const RP_ID: &str = "localhost";

pub struct Job<T> {
    rx: Receiver<T>,
}

/// A job whose worker ended without a result (it panicked). Callers clear the job and show
/// their own generic message; the panic text never reaches the UI.
#[derive(Debug)]
pub struct WorkerDied;

impl<T: Send + 'static> Job<T> {
    pub fn spawn(ctx: &eframe::egui::Context, f: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // A panic is caught (its payload dropped here) so the window still repaints and sees
            // the job end: the sender goes away without a value, which `poll` reports as failed.
            if let Ok(v) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
                let _ = tx.send(v);
            }
            drop(tx);
            ctx.request_repaint();
        });
        Job { rx }
    }

    /// `None` while the job runs, then its result, or `Err(WorkerDied)` if the worker panicked.
    /// A dead job never reads as still running.
    pub fn poll(&self) -> Option<Result<T, WorkerDied>> {
        match self.rx.try_recv() {
            Ok(v) => Some(Ok(v)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(WorkerDied)),
        }
    }

    /// Blocks up to `timeout` for the job to end; `None` if it is still running.
    #[cfg(test)]
    pub fn wait(&self, timeout: std::time::Duration) -> Option<Result<T, WorkerDied>> {
        use std::sync::mpsc::RecvTimeoutError;
        match self.rx.recv_timeout(timeout) {
            Ok(v) => Some(Ok(v)),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => Some(Err(WorkerDied)),
        }
    }
}

/// HWNDs are plain handles; carried across threads as integers.
#[derive(Clone, Copy)]
pub struct WindowHandle(usize);

impl WindowHandle {
    pub fn foreground() -> Self {
        WindowHandle(passkey_win::foreground_window() as usize)
    }

    fn raw(self) -> passkey_win::Hwnd {
        self.0 as passkey_win::Hwnd
    }
}

pub enum KeySource {
    None,
    KeyFile(PathBuf),
    Passkey,
}

pub enum UnlockError {
    Cancelled,
    Failed(String),
    /// The database's header asks for more key derivation than the budget allows. Nothing was
    /// derived; the unlock screen offers to approve exactly these settings.
    HeavyKdf(Kdf),
}

impl UnlockError {
    /// What to show the user.
    pub fn into_message(self) -> String {
        match self {
            UnlockError::Cancelled => "Cancelled.".into(),
            UnlockError::Failed(msg) => msg,
            UnlockError::HeavyKdf(kdf) => heavy_kdf_message(&kdf),
        }
    }
}

pub fn heavy_kdf_message(kdf: &Kdf) -> String {
    format!(
        "This database asks for unusually heavy key derivation ({}), above Axentra Vault's limit ({}). A tampered file can use this to freeze your PC before your password is checked. Unlock anyway only if you chose these settings yourself.",
        kdf.details(),
        KdfBudget::DEFAULT.summary()
    )
}

/// Builds the credentials (asking the passkey if needed) and opens the database.
/// What a running unlock is waiting for (shown in the status line).
pub const PHASE_PASSKEY: u8 = 0;
pub const PHASE_DERIVING: u8 = 1;

/// `kdf_budget` limits the database header's key derivation (`KdfBudget::DEFAULT` unless the
/// user approved this database's heavier settings). `backups` is where the vault keeps copies of
/// the versions it replaces. Once open, on this worker:
/// - the version found on disk is copied there as `opened` (not for a recovered session, whose
///   file was read before);
/// - the store is repaired (a key change interrupted after staging, leftover save temps; see
///   `Vault::recover_backups`);
/// - the passkey file is snapshotted into the store;
/// - retention runs, as after a save (see `Vault::prune_backups`).
///
/// A backup in a vault's folder under a backup root (see `BackupConfig::backup_root_of`) opens
/// read-only, with no store. Other files under a root, such as another vault on the same USB
/// drive, open as vaults.
#[allow(clippy::too_many_arguments)]
pub fn unlock(
    db_path: PathBuf,
    recovery: Option<SealedVault>,
    password: Option<Zeroizing<String>>,
    source: KeySource,
    hwnd: WindowHandle,
    phase: std::sync::Arc<std::sync::atomic::AtomicU8>,
    kdf_budget: KdfBudget,
    backups: BackupConfig,
) -> Result<Vault, UnlockError> {
    unlock_evaluating(
        db_path,
        recovery,
        password,
        source,
        phase,
        kdf_budget,
        backups,
        |rp_id, creds| passkey_win::evaluate_prf_any(hwnd.raw(), rp_id, creds),
    )
}

/// `unlock` with the authenticator call passed in: `evaluate(rp_id, credentials)`.
#[allow(clippy::too_many_arguments)]
fn unlock_evaluating(
    db_path: PathBuf,
    recovery: Option<SealedVault>,
    password: Option<Zeroizing<String>>,
    source: KeySource,
    phase: std::sync::Arc<std::sync::atomic::AtomicU8>,
    kdf_budget: KdfBudget,
    backups: BackupConfig,
    evaluate: impl FnMut(&str, &[PrfCredential]) -> Result<PrfAnswer, PasskeyError>,
) -> Result<Vault, UnlockError> {
    let open = |key_file: Option<Zeroizing<Vec<u8>>>| {
        phase.store(PHASE_DERIVING, std::sync::atomic::Ordering::Relaxed);
        let credentials = Credentials {
            password: password.clone(),
            key_file,
        };
        match &recovery {
            Some(sealed) => Vault::open_sealed(sealed, credentials).map(|v| (v, None)),
            None => Vault::open_read(&db_path, credentials, kdf_budget).map(|(v, o)| (v, Some(o))),
        }
    };
    let (mut vault, opened) = match source {
        KeySource::None => open(None).map_err(open_error)?,
        KeySource::KeyFile(p) => {
            let key_file = limits::read_capped(&p, limits::KEY_FILE_MAX)
                .map_err(|e| UnlockError::Failed(format!("Key file: {e}")))?;
            open(Some(key_file)).map_err(open_error)?
        }
        KeySource::Passkey => {
            unlock_with_passkey_with(&db_path, &backups, evaluate, |key| open(Some(key)))?
        }
    };
    if vault.is_read_only() || backups.backup_root_of(vault.path()).is_some() {
        vault.set_read_only();
        return Ok(vault);
    }
    vault.attach_store(backups);
    // None of these fails the unlock: a store that can't be written leaves the vault's notice.
    if let Some(opened) = opened {
        let _ = vault.snapshot_opened(&opened);
    }
    let _ = vault.recover_backups();
    let _ = vault.snapshot_sidecar();
    vault.prune_backups();
    Ok(vault)
}

fn open_error(e: VaultError) -> UnlockError {
    match e {
        VaultError::KdfOverBudget(kdf) => UnlockError::HeavyKdf(kdf),
        e => UnlockError::Failed(e.to_string()),
    }
}

/// The PRF output of whichever passkey the user picked, and that passkey's index in the list.
type PrfAnswer = (usize, Zeroizing<[u8; 32]>);

/// A PRF secret one of this unlock's prompts returned: the credential, its PRF input, the output.
type KnownPrf = (Vec<u8>, [u8; 32], Zeroizing<[u8; 32]>);

/// The database key `ring` wraps, from a PRF secret already known for one of its keys, or else
/// by asking (`evaluate`) for the keys of this app's relying party not asked yet. Never asks
/// twice for the same key.
fn unwrap_ring(
    ring: &KeyRing,
    env_path: &Path,
    evaluate: &mut impl FnMut(&str, &[PrfCredential]) -> Result<PrfAnswer, PasskeyError>,
    known: &mut Vec<KnownPrf>,
) -> Result<Zeroizing<[u8; 32]>, UnlockError> {
    let wrong_key = || {
        UnlockError::Failed("That key does not unlock this database. Use one registered for it, or your backup key file.".into())
    };
    // Only this app's relying party: the file can't steer the prompt to another site's passkeys.
    let keys: Vec<&RingKey> = ring
        .keys
        .iter()
        .filter(|k| k.envelope.rp_id == RP_ID)
        .collect();
    if keys.is_empty() {
        return Err(UnlockError::Failed(format!(
            "The passkey file {} has no key for Axentra Vault. Use your backup key file.",
            env_path.display()
        )));
    }
    let is_known = |k: &RingKey, (id, input, _): &KnownPrf| {
        *id == k.envelope.credential_id && *input == k.envelope.prf_input
    };
    for k in &keys {
        if let Some((_, _, prf)) = known.iter().find(|p| is_known(k, p))
            && let Ok(key) = k.envelope.open(prf)
        {
            return Ok(key);
        }
    }
    let ask: Vec<&RingKey> = keys
        .into_iter()
        .filter(|k| !known.iter().any(|p| is_known(k, p)))
        .collect();
    if ask.is_empty() {
        return Err(wrong_key());
    }
    let creds: Vec<PrfCredential> = ask
        .iter()
        .map(|k| PrfCredential {
            credential_id: &k.envelope.credential_id,
            prf_input: &k.envelope.prf_input,
        })
        .collect();
    let (used, prf) = evaluate(RP_ID, &creds).map_err(passkey_error)?;
    let k = ask.get(used).ok_or_else(wrong_key)?;
    known.push((k.envelope.credential_id.clone(), k.envelope.prf_input, prf));
    let prf = &known.last().expect("just added").2;
    k.envelope.open(prf).map_err(|_| wrong_key())
}

/// Passkey unlock: the key the passkey file wraps for the passkey the user picks, opened with
/// `open`. If the file is missing, unreadable or its key doesn't open the database, a ring a key
/// change left behind is tried: the staged `<db>.axvault.~new` (the app stopped between saving
/// the database and moving the ring into place), the `<db>.axvault.old` earlier versions and
/// the KeePass plugin keep (an enrollment abandoned before the database was saved), then the
/// newest ring snapshots in the store. A ring counts only if its key opens the database. It
/// then replaces the passkey file only if that file is missing or was read and its key doesn't
/// open the database: a file that couldn't be read just now (being synced, held by antivirus)
/// may hold keys added since, so it is left as it is. Keys already tried are not tried again,
/// and a passkey already asked is not asked again.
fn unlock_with_passkey_with<T>(
    db_path: &Path,
    backups: &BackupConfig,
    mut evaluate: impl FnMut(&str, &[PrfCredential]) -> Result<PrfAnswer, PasskeyError>,
    mut open: impl FnMut(Zeroizing<Vec<u8>>) -> Result<T, VaultError>,
) -> Result<T, UnlockError> {
    let env_path = envelope::path_for(db_path);
    let mut known: Vec<KnownPrf> = Vec::new();
    let mut tried: Vec<Zeroizing<[u8; 32]>> = Vec::new();
    let read = |path: &Path| -> Result<(String, KeyRing), UnlockError> {
        let text = envelope::read_text(path).map_err(|e| {
            UnlockError::Failed(if e.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "No passkey file at {}. Use your backup key file.",
                    path.display()
                )
            } else {
                format!(
                    "Could not read the passkey file {} ({e}). Use your backup key file.",
                    path.display()
                )
            })
        })?;
        let ring = KeyRing::parse(&text).map_err(|e| UnlockError::Failed(e.to_string()))?;
        Ok((text, ring))
    };

    let staged = envelope::staged_path_for(db_path);
    // Whether the passkey file is known to be stale (or missing), so a ring that works may
    // replace it.
    let (first_error, stale) = match read(&env_path)
        .and_then(|(_, ring)| unwrap_ring(&ring, &env_path, &mut evaluate, &mut known))
    {
        Ok(key) => match open(Zeroizing::new(key.to_vec())) {
            Ok(opened) => {
                // The passkey file is current, so a staged ring is left from a key change that
                // never committed.
                let _ = vault_core::fsops::remove_file(&staged);
                return Ok(opened);
            }
            Err(e) if e.is_wrong_key() => {
                tried.push(key);
                (open_error(e), true)
            }
            Err(e) => return Err(open_error(e)),
        },
        Err(UnlockError::Cancelled) => return Err(UnlockError::Cancelled),
        Err(e) => (
            e,
            std::fs::symlink_metadata(&env_path)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        ),
    };

    let candidates = [staged.clone(), envelope::legacy_old_path_for(db_path)]
        .into_iter()
        .chain(backups.ring_snapshots_for(db_path).into_iter().take(3));
    for path in candidates {
        let Ok((text, ring)) = read(&path) else {
            continue;
        };
        let key = match unwrap_ring(&ring, &path, &mut evaluate, &mut known) {
            Ok(key) => key,
            Err(UnlockError::Cancelled) => return Err(UnlockError::Cancelled),
            Err(_) => continue,
        };
        if tried.iter().any(|t| **t == *key) {
            continue;
        }
        match open(Zeroizing::new(key.to_vec())) {
            Ok(opened) => {
                // Opened: this ring holds the current key. Failing to put it back only means
                // the next passkey unlock recovers it again.
                if stale {
                    let _ = if path == staged {
                        vault_core::fsops::move_replace(&staged, &env_path)
                    } else {
                        envelope::write_ring_text(&env_path, &text)
                    };
                }
                return Ok(opened);
            }
            Err(e) if e.is_wrong_key() => tried.push(key),
            Err(e) => return Err(open_error(e)),
        }
    }
    Err(first_error)
}

fn passkey_error(e: PasskeyError) -> UnlockError {
    if e.is_cancellation() {
        UnlockError::Cancelled
    } else if e.name == "InvalidStateError" {
        UnlockError::Failed(
            "That key is already set up for this vault. Use a different phone or security key."
                .into(),
        )
    } else {
        UnlockError::Failed(e.message)
    }
}

/// Result of registering a passkey or security key for a database: the sealed envelope, the
/// raw key it wraps, and a label for the kind of authenticator used.
pub struct NewPasskey {
    pub envelope: Envelope,
    pub key: Zeroizing<[u8; 32]>,
    pub label: String,
    /// True when it wraps the database's current key (another unlock key), false for a fresh key.
    pub added: bool,
}

/// The database key an added unlock key must wrap, plus the keys already registered.
pub struct ExistingKey {
    /// The current key file's effective KeePass key, as a raw 32-byte key file.
    pub key: Zeroizing<[u8; 32]>,
    /// The PRF input the other keys use, so one Windows prompt can ask any of them.
    pub prf_input: Option<[u8; 32]>,
    pub registered: Vec<Vec<u8>>,
    /// The database and master password that, with `key`, must open before anything is registered.
    pub db_path: PathBuf,
    pub password: Option<Zeroizing<String>>,
    /// The open vault's KDF budget, which that check stays within.
    pub kdf_budget: KdfBudget,
}

/// Registers a passkey or security key (first prompt), then evaluates PRF to wrap the key (second
/// prompt): the database's current key when `existing` is given, otherwise a fresh random one.
pub fn register_passkey(
    db_name: String,
    hwnd: WindowHandle,
    existing: Option<ExistingKey>,
) -> Result<NewPasskey, UnlockError> {
    register_passkey_with(
        existing,
        |user_id, exclude| {
            passkey_win::register(
                hwnd.raw(),
                RP_ID,
                "Axentra Vault",
                user_id,
                &format!("Axentra Vault: {db_name}"),
                exclude,
            )
        },
        |credential_id, prf_input| {
            passkey_win::evaluate_prf(hwnd.raw(), RP_ID, credential_id, prf_input)
        },
    )
}

/// `register_passkey` with the two authenticator calls passed in: `register(user_id, exclude)`
/// and `evaluate(credential_id, prf_input)`.
fn register_passkey_with(
    existing: Option<ExistingKey>,
    register: impl FnOnce(&[u8], &[&[u8]]) -> Result<Registration, PasskeyError>,
    evaluate: impl FnOnce(&[u8], &[u8; 32]) -> Result<Zeroizing<[u8; 32]>, PasskeyError>,
) -> Result<NewPasskey, UnlockError> {
    // An added key must wrap a key that really opens this database. Check before the
    // authenticator stores a credential that could never unlock it.
    if let Some(e) = &existing {
        let candidate = Credentials {
            password: e.password.clone(),
            key_file: Some(Zeroizing::new(e.key.to_vec())),
        };
        Vault::verify_credentials(&e.db_path, &candidate, &e.kdf_budget).map_err(|err| {
            UnlockError::Failed(format!(
                "Could not confirm that a passkey would unlock this database ({err}). Nothing was registered; lock and reopen the vault, then try again."
            ))
        })?;
    }
    let mut user_id = [0u8; 16];
    random_bytes(&mut user_id);
    let exclude: Vec<&[u8]> = existing
        .iter()
        .flat_map(|e| e.registered.iter().map(Vec::as_slice))
        .collect();
    let reg = register(&user_id, &exclude).map_err(passkey_error)?;
    let label = passkey_win::transport_label(reg.transport);
    let added = existing.is_some();
    let (key, prf_input) = match existing {
        Some(e) => {
            let input = e.prf_input.unwrap_or_else(|| {
                let mut i = [0u8; 32];
                random_bytes(&mut i);
                i
            });
            (e.key, input)
        }
        None => {
            let mut key = Zeroizing::new([0u8; 32]);
            random_bytes(&mut key[..]);
            let mut input = [0u8; 32];
            random_bytes(&mut input);
            (key, input)
        }
    };
    // Windows' PRF flag is only a hint (security keys may not report it), so ask for the PRF
    // secret either way; the key's answer is what decides.
    let prf = evaluate(&reg.credential_id, &prf_input)
        .map_err(|e| {
            if e.is_cancellation() {
                UnlockError::Cancelled
            } else if e.name == "NotSupportedError" {
                UnlockError::Failed(format!(
                    "This key ({label}{}) did not return a PRF secret, so it cannot unlock a database. Delete the passkey just created on it and use an iPhone, Android phone, or FIDO2 security key with hmac-secret (YubiKey 5 / Security Key series).",
                    if reg.prf_enabled { "" } else { ", PRF not reported at registration" }
                ))
            } else {
                passkey_error(e)
            }
        })?;

    let envelope = Envelope::seal(RP_ID, &reg.credential_id, prf_input, &prf, &key);
    if *envelope
        .open(&prf)
        .map_err(|e| UnlockError::Failed(e.to_string()))?
        != *key
    {
        return Err(UnlockError::Failed("Passkey self-check failed.".into()));
    }
    Ok(NewPasskey {
        envelope,
        key,
        label: label.to_owned(),
        added,
    })
}

/// A completed rotation contains no old database key. No files are changed by the worker.
pub struct RotatedKeys {
    pub ring: KeyRing,
    pub key: Zeroizing<[u8; 32]>,
}

pub fn rotate_keys(
    ring: KeyRing,
    remove: usize,
    current_key: Zeroizing<[u8; 32]>,
    hwnd: WindowHandle,
) -> Result<RotatedKeys, UnlockError> {
    rotate_keys_with(&ring, remove, &current_key, |env| {
        passkey_win::evaluate_prf(hwnd.raw(), &env.rp_id, &env.credential_id, &env.prf_input)
            .map_err(passkey_error)
    })
}

fn rotate_keys_with(
    ring: &KeyRing,
    remove: usize,
    current_key: &[u8; 32],
    mut evaluate: impl FnMut(&Envelope) -> Result<Zeroizing<[u8; 32]>, UnlockError>,
) -> Result<RotatedKeys, UnlockError> {
    if ring.keys.len() < 2 || remove >= ring.keys.len() {
        return Err(UnlockError::Failed(
            "At least one retained unlock key is required.".into(),
        ));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    random_bytes(&mut key[..]);
    let mut keys = Vec::new();
    for (index, old) in ring.keys.iter().enumerate() {
        if index == remove {
            continue;
        }
        if old.envelope.rp_id != RP_ID {
            return Err(UnlockError::Failed(
                "Unexpected relying-party ID in the key ring.".into(),
            ));
        }
        let prf = evaluate(&old.envelope)?;
        let opened = old
            .envelope
            .open(&prf)
            .map_err(|e| UnlockError::Failed(e.to_string()))?;
        if &*opened != current_key {
            return Err(UnlockError::Failed(
                "An unlock key does not match the open database.".into(),
            ));
        }
        keys.push(RingKey {
            label: old.label.clone(),
            envelope: Envelope::seal(
                RP_ID,
                &old.envelope.credential_id,
                old.envelope.prf_input,
                &prf,
                &key,
            ),
        });
    }
    Ok(RotatedKeys {
        ring: KeyRing { keys },
        key,
    })
}

#[cfg(test)]
mod job_tests {
    use super::*;
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(10);

    #[test]
    fn a_panicking_job_reports_failure() {
        let ctx = eframe::egui::Context::default();
        let ok = Job::spawn(&ctx, || 7);
        assert!(matches!(ok.wait(TIMEOUT), Some(Ok(7))));

        let dead = Job::spawn(&ctx, || -> u32 { panic!("job panic canary") });
        assert!(
            matches!(dead.wait(TIMEOUT), Some(Err(WorkerDied))),
            "a panicked worker ends the job as failed"
        );
        assert!(
            matches!(dead.poll(), Some(Err(WorkerDied))),
            "and it never reads as running again"
        );
    }
}

#[cfg(test)]
mod rotation_tests {
    use super::*;
    use std::fs;
    use vault_core::backups::{self, BackupConfig, BackupSettings};
    #[test]
    fn revocation_rotates_retained_keys_and_rejects_old_envelopes() {
        let old_key = [42; 32];
        let ring = KeyRing {
            keys: (1..=3)
                .map(|id| RingKey {
                    label: format!("key {id}"),
                    envelope: Envelope::seal(RP_ID, &[id], [9; 32], &[id; 32], &old_key),
                })
                .collect(),
        };
        let rotated = rotate_keys_with(&ring, 0, &old_key, |env| {
            Ok(Zeroizing::new([env.credential_id[0]; 32]))
        })
        .unwrap_or_else(|_| panic!("rotation failed"));
        assert_ne!(*rotated.key, old_key);
        assert_eq!(rotated.ring.keys.len(), 2);
        for key in &rotated.ring.keys {
            let prf = [key.envelope.credential_id[0]; 32];
            assert_eq!(*key.envelope.open(&prf).unwrap(), *rotated.key);
        }
        let dir = std::env::temp_dir().join(format!("axv-rotation-{}", uuid_for_test()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rotation.kdbx");
        let credentials = |key: &[u8]| Credentials {
            password: Some(Zeroizing::new("test".into())),
            key_file: Some(Zeroizing::new(key.to_vec())),
        };
        let mut db = vault_core::keepass::Database::new();
        db.config.kdf_config = vault_core::keepass::config::KdfConfig::Aes { rounds: 1000 };
        db.save(
            &mut fs::File::create(&path).unwrap(),
            credentials(&old_key).to_key().unwrap(),
        )
        .unwrap();
        let config = BackupConfig::new(dir.join("Backups"));
        let mut vault = Vault::open(&path, credentials(&old_key)).unwrap();
        vault.attach_store(config.clone());
        vault.set_credentials(credentials(&*rotated.key));
        vault.save().unwrap();
        let stolen_old = ring.keys[0].envelope.open(&[1; 32]).unwrap();
        assert!(Vault::open(&path, credentials(&*stolen_old)).is_err());
        // The previous version went to the store, re-encrypted to the new key; nothing beside
        // the database.
        let (folders, _) = config.existing_folders(
            &path,
            &backups::vault_id_of(&vault.db),
            &mut BackupSettings::default(),
        );
        let copies: Vec<_> = folders.iter().flat_map(|f| f.list().unwrap()).collect();
        assert_eq!(copies.len(), 1);
        assert!(Vault::open(&copies[0].path, credentials(&*stolen_old)).is_err());
        assert!(Vault::open(&copies[0].path, credentials(&*rotated.key)).is_ok());
        assert!(!vault_core::vault::with_suffix(&path, ".bak").exists());
        assert!(Vault::open(&path, credentials(&*rotated.key)).is_ok());
        assert!(
            Vault::open(
                &path,
                Credentials {
                    password: Some(Zeroizing::new("test".into())),
                    key_file: Some(Zeroizing::new(
                        vault_core::keyfile::build_v2(&rotated.key).into_bytes()
                    )),
                }
            )
            .is_ok()
        );
    }
    fn uuid_for_test() -> String {
        let mut bytes = [0; 16];
        random_bytes(&mut bytes);
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Re-encrypts the backups to `new` and commits the key change with `ring`.
    fn change_key(vault: &mut Vault, new: Credentials, ring: &KeyRing) {
        use std::sync::atomic::AtomicBool;
        let staged = vault
            .plan_key_change(new)
            .unwrap_or_else(|e| panic!("{e}"))
            .run(&AtomicBool::new(false), &Default::default())
            .unwrap();
        vault
            .commit_key_change(staged, ring, Some("rotation-backup.keyx"))
            .unwrap();
    }

    /// Nothing this app can reach opens with `old`: no KeePass file under a known root, at
    /// `<db>.bak` or in a `<db>…tmp` beside the database; and no passkey file beside it or in
    /// the store lists a `revoked` key.
    fn assert_nothing_reachable_opens(
        path: &Path,
        config: &BackupConfig,
        old: &[Credentials],
        revoked: &[&[u8]],
    ) {
        let db_name = path.file_name().unwrap().to_string_lossy().into_owned();
        let (mut databases, mut rings) = (Vec::new(), Vec::new());
        let mut stack: Vec<PathBuf> = config.known_roots().into_iter().map(|r| r.path).collect();
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let p = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if p.is_dir() {
                    stack.push(p);
                } else if name.ends_with(".axvault") {
                    rings.push(p);
                } else if name.ends_with(".kdbx") || name.ends_with(".tmp") {
                    databases.push(p);
                }
            }
        }
        assert!(databases.len() >= 5, "{databases:?}");
        for entry in fs::read_dir(path.parent().unwrap()).unwrap().flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == db_name {
                continue;
            }
            if name.starts_with(&format!("{db_name}{}", envelope::EXTENSION)) {
                rings.push(p);
            } else if name.starts_with(&db_name)
                && (name.ends_with(".tmp") || name.ends_with(".bak"))
            {
                databases.push(p);
            }
        }
        for file in &databases {
            for c in old {
                assert!(
                    Vault::open(file, c.clone()).is_err(),
                    "{} opens with credentials that were replaced",
                    file.display()
                );
            }
        }
        assert!(!rings.is_empty());
        for ring in &rings {
            let parsed = KeyRing::parse(&fs::read_to_string(ring).unwrap()).unwrap();
            for id in revoked {
                assert!(
                    parsed.find(id).is_none(),
                    "{} lists a revoked key",
                    ring.display()
                );
            }
        }
    }

    /// After a key change nothing reachable opens with the credentials it replaced: for a first
    /// passkey setup (password alone → password + K1), then for a revocation (K1 → K2), with a
    /// `.bak` beside the database, a leftover save temp and the `.axvault.old` that earlier
    /// versions kept.
    #[test]
    fn after_key_change_nothing_reachable_opens_with_old_credentials() {
        let dir = std::env::temp_dir().join(format!("axv-reachable-{}", uuid_for_test()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rotation.kdbx");
        let credentials = |key: Option<&[u8; 32]>| Credentials {
            password: Some(Zeroizing::new("test".into())),
            key_file: key.map(|k| Zeroizing::new(k.to_vec())),
        };
        let password_only = credentials(None);
        let mut db = vault_core::keepass::Database::new();
        db.config.kdf_config = vault_core::keepass::config::KdfConfig::Aes { rounds: 1000 };
        db.save(
            &mut fs::File::create(&path).unwrap(),
            password_only.to_key().unwrap(),
        )
        .unwrap();
        let config = BackupConfig::load(dir.join("Backups"), dir.join(backups::SETTINGS_FILE))
            .with_free_space(|_| None);
        let mut vault = Vault::open(&path, password_only.clone()).unwrap();
        vault.attach_store(config.clone());
        let mut earlier = Vec::new();
        for i in 0..3 {
            earlier = fs::read(&path).unwrap();
            vault
                .db
                .root_mut()
                .add_entry()
                .set_unprotected(vault_core::keepass::db::fields::TITLE, format!("e{i}"));
            vault.mark_dirty();
            vault.save().unwrap();
        }
        // A save while no backup folder worked kept the previous version as `.bak` (to be moved
        // in once the folder is back), and a save temp a crash left, which the next unlock takes
        // into the store.
        let store = dir.join("Backups");
        let away = dir.join("Backups.away");
        fs::rename(&store, &away).unwrap();
        fs::write(&store, b"not a folder").unwrap();
        vault.mark_dirty();
        vault.save().unwrap();
        let bak = vault_core::vault::with_suffix(&path, ".bak");
        assert!(bak.is_file(), "the fallback .bak");
        fs::remove_file(&store).unwrap();
        fs::rename(&away, &store).unwrap();
        let leftover = dir.join(format!("rotation.kdbx{}", vault_core::fsops::temp_name()));
        fs::write(&leftover, &earlier).unwrap();
        assert_eq!(vault.recover_backups().imported, [leftover.as_path()]);

        // First passkey setup: a fresh key, wrapped by the new passkey.
        let Ok(new) = register_passkey_with(
            None,
            |_, _| {
                Ok(Registration {
                    credential_id: vec![1],
                    prf_enabled: true,
                    transport: passkey_win::TRANSPORT_USB,
                })
            },
            |_, _| Ok(Zeroizing::new([1; 32])),
        ) else {
            panic!("registration failed");
        };
        let k1 = *new.key;
        let ring = KeyRing {
            keys: vec![RingKey {
                label: new.label,
                envelope: new.envelope,
            }],
        };
        change_key(&mut vault, credentials(Some(&k1)), &ring);
        assert!(!bak.exists() && !leftover.exists());
        assert_nothing_reachable_opens(&path, &config, std::slice::from_ref(&password_only), &[]);

        // Two more keys join; an earlier version kept a copy of that ring as `.axvault.old`.
        let ring = KeyRing {
            keys: (1..=3)
                .map(|id| RingKey {
                    label: format!("key {id}"),
                    envelope: Envelope::seal(RP_ID, &[id], [9; 32], &[id; 32], &k1),
                })
                .collect(),
        };
        let text = envelope::write_ring(&envelope::path_for(&path), &ring).unwrap();
        vault.snapshot_ring(text.as_bytes()).unwrap();
        fs::write(envelope::legacy_old_path_for(&path), &text).unwrap();

        // Key 1 is revoked.
        let rotated = rotate_keys_with(&ring, 0, &k1, |env| {
            Ok(Zeroizing::new([env.credential_id[0]; 32]))
        })
        .unwrap_or_else(|_| panic!("rotation failed"));
        change_key(&mut vault, credentials(Some(&rotated.key)), &rotated.ring);
        assert!(!envelope::legacy_old_path_for(&path).exists());
        assert_nothing_reachable_opens(
            &path,
            &config,
            &[password_only, credentials(Some(&k1))],
            &[&[1]],
        );
        assert!(Vault::open(&path, credentials(Some(&rotated.key))).is_ok());
        drop(vault);
        clear_readonly_and_remove(&dir);
    }

    fn clear_readonly_and_remove(dir: &Path) {
        fn clear(dir: &Path) {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    clear(&path);
                } else {
                    let _ = vault_core::fsops::set_readonly(&path, false);
                }
            }
        }
        clear(dir);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn cancelled_rotation_does_not_change_the_ring() {
        let ring = KeyRing {
            keys: (1..=3)
                .map(|id| RingKey {
                    label: format!("key {id}"),
                    envelope: Envelope::seal(RP_ID, &[id], [9; 32], &[id; 32], &[42; 32]),
                })
                .collect(),
        };
        let before = ring.serialize();
        let result = rotate_keys_with(&ring, 0, &[42; 32], |env| {
            if env.credential_id == [3] {
                Err(UnlockError::Cancelled)
            } else {
                Ok(Zeroizing::new([2; 32]))
            }
        });
        assert!(matches!(result, Err(UnlockError::Cancelled)));
        assert_eq!(ring.serialize(), before);
    }
}

#[cfg(test)]
mod registration_tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use vault_core::keepass::{Database, config::KdfConfig};
    use vault_core::keyfile;

    const PASSWORD: &str = "registration-master";

    fn password() -> Option<Zeroizing<String>> {
        Some(Zeroizing::new(PASSWORD.into()))
    }

    /// A synthetic database in its own temp folder, keyed with the password plus `key_file`.
    fn database(key_file: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("axv-register-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("synthetic.kdbx");
        let credentials = Credentials {
            password: password(),
            key_file: Some(Zeroizing::new(key_file.to_vec())),
        };
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
        db.save(
            &mut fs::File::create(&path).unwrap(),
            credentials.to_key().unwrap(),
        )
        .unwrap();
        path
    }

    fn existing(path: &Path, key: [u8; 32]) -> ExistingKey {
        ExistingKey {
            key: Zeroizing::new(key),
            prf_input: Some([3; 32]),
            registered: vec![vec![1]],
            db_path: path.to_owned(),
            password: password(),
            kdf_budget: KdfBudget::DEFAULT,
        }
    }

    fn registration() -> Registration {
        Registration {
            credential_id: vec![2],
            prf_enabled: true,
            transport: passkey_win::TRANSPORT_USB,
        }
    }

    /// The audit's M5 key file: 64 hex digits plus a newline, which KeePass hashes whole.
    fn hex_with_newline() -> Vec<u8> {
        format!("{}\n", "ab".repeat(32)).into_bytes()
    }

    #[test]
    fn added_key_wraps_the_key_files_effective_key() {
        let file = hex_with_newline();
        let path = database(&file);
        let key = keyfile::raw_key(&file).unwrap();
        let prf = [77; 32];
        let Ok(new) = register_passkey_with(
            Some(existing(&path, key)),
            |_, exclude| {
                assert_eq!(
                    exclude.to_vec(),
                    vec![&[1u8][..]],
                    "already registered keys"
                );
                Ok(registration())
            },
            |credential_id, input| {
                assert_eq!(credential_id, &[2u8][..]);
                assert_eq!(*input, [3; 32], "the ring's shared PRF input");
                Ok(Zeroizing::new(prf))
            },
        ) else {
            panic!("registration failed");
        };
        assert!(new.added);
        let unwrapped = new.envelope.open(&prf).unwrap();
        assert!(
            Vault::open(
                &path,
                Credentials {
                    password: password(),
                    key_file: Some(Zeroizing::new(unwrapped.to_vec())),
                },
            )
            .is_ok(),
            "password + the unwrapped key opens the database"
        );
        fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_key_that_does_not_open_the_database_is_never_registered() {
        let path = database(&hex_with_newline());
        // What the old trimming parser wrapped: the decoded hex, which KeePass never used.
        let wrong = [0xab; 32];
        let (registered, evaluated) = (Cell::new(false), Cell::new(false));
        let result = register_passkey_with(
            Some(existing(&path, wrong)),
            |_, _| {
                registered.set(true);
                Ok(registration())
            },
            |_, _| {
                evaluated.set(true);
                Ok(Zeroizing::new([0; 32]))
            },
        );
        assert!(matches!(result, Err(UnlockError::Failed(_))));
        assert!(
            !registered.get() && !evaluated.get(),
            "no credential is created"
        );

        // A missing database fails the same way, before any prompt.
        fs::remove_file(&path).unwrap();
        let key = keyfile::raw_key(&hex_with_newline()).unwrap();
        let result = register_passkey_with(
            Some(existing(&path, key)),
            |_, _| {
                registered.set(true);
                Ok(registration())
            },
            |_, _| Ok(Zeroizing::new([0; 32])),
        );
        assert!(matches!(result, Err(UnlockError::Failed(_))));
        assert!(!registered.get());

        // First-time setup wraps a fresh key and has nothing to check.
        let Ok(fresh) = register_passkey_with(
            None,
            |_, exclude| {
                assert!(exclude.is_empty());
                registered.set(true);
                Ok(registration())
            },
            |_, _| Ok(Zeroizing::new([5; 32])),
        ) else {
            panic!("first-time registration failed");
        };
        assert!(registered.get() && !fresh.added);
        fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}

#[cfg(test)]
mod unlock_tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("axv-limits-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn failure<T>(result: Result<T, UnlockError>) -> String {
        match result {
            Err(UnlockError::Failed(msg)) => msg,
            Err(UnlockError::Cancelled) => panic!("cancelled instead of failed"),
            Err(UnlockError::HeavyKdf(kdf)) => panic!("refused for its KDF: {}", kdf.details()),
            Ok(_) => panic!("expected a failure"),
        }
    }

    /// Backups go to a `Backups` folder beside the test database.
    fn backups_beside(db: &Path) -> BackupConfig {
        BackupConfig::new(db.with_file_name("Backups"))
    }

    /// Password-only unlock of `db` within `budget`.
    fn unlock_password(db: &Path, budget: KdfBudget) -> Result<Vault, UnlockError> {
        unlock_with(db, None, budget)
    }

    fn unlock_with(
        db: &Path,
        recovery: Option<SealedVault>,
        budget: KdfBudget,
    ) -> Result<Vault, UnlockError> {
        unlock(
            db.to_owned(),
            recovery,
            Some(Zeroizing::new("pw".into())),
            KeySource::None,
            WindowHandle(0),
            Default::default(),
            budget,
            backups_beside(db),
        )
    }

    fn unlocked(result: Result<Vault, UnlockError>) -> Vault {
        result.unwrap_or_else(|e| panic!("unlock failed: {}", e.into_message()))
    }

    /// The key passkey unlock of `db` arrives at (any key "opens" it).
    fn passkey_key(
        db: &Path,
        evaluate: impl FnMut(&str, &[PrfCredential]) -> Result<PrfAnswer, PasskeyError>,
    ) -> Result<Zeroizing<Vec<u8>>, UnlockError> {
        unlock_with_passkey_with(db, &backups_beside(db), evaluate, Ok)
    }

    /// A database opened by the password "pw" plus the raw 32-byte key file `key`.
    fn keyed_db(path: &Path, key: [u8; 32]) {
        use vault_core::keepass::{Database, config::KdfConfig};
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
        let credentials = Credentials {
            password: Some(Zeroizing::new("pw".into())),
            key_file: Some(Zeroizing::new(key.to_vec())),
        };
        db.save(
            &mut fs::File::create(path).unwrap(),
            credentials.to_key().unwrap(),
        )
        .unwrap();
    }

    /// The app stopped after a key change saved the database on K2 and before the new ring
    /// replaced the passkey file, which still wraps K1. Passkey unlock finds that K1 doesn't
    /// open the database and takes the staged `<db>.axvault.~new` instead (the same passkey, so
    /// no second prompt), then puts it in place. After a first setup that stopped the same way
    /// there is no passkey file yet, and the staged ring is asked for. Without the staged ring
    /// the newest ring snapshot in the store serves. A ring is used only if it opens the file.
    #[test]
    fn staged_ring_recovers_passkey_unlock_after_crash() {
        let dir = temp_dir();
        let db = dir.join("synthetic.kdbx");
        let (k1, k2, prf) = ([1u8; 32], [2u8; 32], [7u8; 32]);
        keyed_db(&db, k2);
        let ring_for = |key: &[u8; 32]| {
            KeyRing {
                keys: vec![RingKey {
                    label: "Phone passkey".into(),
                    envelope: Envelope::seal(RP_ID, &[1], [2; 32], &prf, key),
                }],
            }
            .serialize()
        };
        let (stale, current) = (ring_for(&k1), ring_for(&k2));
        let (sidecar, staged) = (envelope::path_for(&db), envelope::staged_path_for(&db));
        let prompts = Cell::new(0);
        let evaluate = |_: &str, creds: &[PrfCredential]| {
            prompts.set(prompts.get() + 1);
            assert_eq!(creds.len(), 1);
            Ok((0, Zeroizing::new(prf)))
        };
        let passkey_unlock = |config: BackupConfig| {
            prompts.set(0);
            unlock_evaluating(
                db.clone(),
                None,
                Some(Zeroizing::new("pw".into())),
                KeySource::Passkey,
                Default::default(),
                KdfBudget::DEFAULT,
                config,
                evaluate,
            )
        };

        // A revocation that stopped before the ring moved.
        fs::write(&sidecar, &stale).unwrap();
        fs::write(&staged, &current).unwrap();
        drop(unlocked(passkey_unlock(backups_beside(&db))));
        assert_eq!(prompts.get(), 1, "the same passkey opens the staged ring");
        assert_eq!(fs::read_to_string(&sidecar).unwrap(), current, "repaired");
        assert!(!staged.exists());

        // A first setup that stopped the same way.
        fs::remove_file(&sidecar).unwrap();
        fs::write(&staged, &current).unwrap();
        drop(unlocked(passkey_unlock(backups_beside(&db))));
        assert_eq!(prompts.get(), 1);
        assert_eq!(fs::read_to_string(&sidecar).unwrap(), current);
        assert!(!staged.exists());

        // A stale passkey file and nothing to recover from: the usual error.
        fs::write(&sidecar, &stale).unwrap();
        let msg = failure(passkey_unlock(backups_beside(&db)));
        assert!(msg.contains("wrong master password or key"), "{msg}");
        assert_eq!(fs::read_to_string(&sidecar).unwrap(), stale);

        // With settings that map the database to its store folder, the ring snapshot taken at an
        // earlier unlock (with the backup key file, while the passkey file was current) serves.
        let config = BackupConfig::load(dir.join("Backups"), dir.join("backups.json"));
        fs::write(&sidecar, &current).unwrap();
        let key_file = dir.join("backup.key");
        fs::write(&key_file, k2).unwrap();
        let opened = unlocked(unlock_evaluating(
            db.clone(),
            None,
            Some(Zeroizing::new("pw".into())),
            KeySource::KeyFile(key_file),
            Default::default(),
            KdfBudget::DEFAULT,
            config.clone(),
            evaluate,
        ));
        let snapshots = config.ring_snapshots_for(&db);
        assert!(!snapshots.is_empty());
        assert_eq!(fs::read_to_string(&snapshots[0]).unwrap(), current);
        drop(opened);
        fs::write(&sidecar, &stale).unwrap();
        drop(unlocked(passkey_unlock(config)));
        assert_eq!(prompts.get(), 1);
        assert_eq!(fs::read_to_string(&sidecar).unwrap(), current, "repaired");

        // A staged ring from a key change that never committed goes once the passkey file
        // proves current.
        fs::write(&staged, &stale).unwrap();
        drop(unlocked(passkey_unlock(backups_beside(&db))));
        assert!(!staged.exists());
        assert_eq!(fs::read_to_string(&sidecar).unwrap(), current);
        clear_and_remove(&dir);
    }

    /// A passkey file that can't be read just now (caught half written by a sync) isn't
    /// replaced by an older ring snapshot that happens to open the database: it may hold keys
    /// added since. One the KeePass plugin left stale (an enrollment abandoned before KeePass
    /// saved the database) is put right from the `.axvault.old` the plugin kept.
    #[test]
    fn passkey_unlock_replaces_the_passkey_file_only_when_it_is_stale() {
        let dir = temp_dir();
        let db = dir.join("synthetic.kdbx");
        let (k1, k2, prf) = ([1u8; 32], [2u8; 32], [7u8; 32]);
        keyed_db(&db, k1);
        let ring_for = |key: &[u8; 32], ids: &[u8]| {
            KeyRing {
                keys: ids
                    .iter()
                    .map(|&id| RingKey {
                        label: format!("Key {id}"),
                        envelope: Envelope::seal(RP_ID, &[id], [2; 32], &prf, key),
                    })
                    .collect(),
            }
            .serialize()
        };
        let evaluate = |_: &str, _: &[PrfCredential]| Ok((0, Zeroizing::new(prf)));
        let config = BackupConfig::load(dir.join("Backups"), dir.join("backups.json"));
        let unlock_with = |source: KeySource| {
            unlocked(unlock_evaluating(
                db.clone(),
                None,
                Some(Zeroizing::new("pw".into())),
                source,
                Default::default(),
                KdfBudget::DEFAULT,
                config.clone(),
                evaluate,
            ))
        };
        let sidecar = envelope::path_for(&db);

        // An unlock with the backup key file snapshots the ring as it is then: one key.
        fs::write(&sidecar, ring_for(&k1, &[1])).unwrap();
        let key_file = dir.join("backup.key");
        fs::write(&key_file, k1).unwrap();
        drop(unlock_with(KeySource::KeyFile(key_file)));
        assert_eq!(config.ring_snapshots_for(&db).len(), 1);

        // A second key was added since, on another PC; the file is half synced just now.
        let current = ring_for(&k1, &[1, 2]);
        let half = &current[..current.len() / 2];
        fs::write(&sidecar, half).unwrap();
        drop(unlock_with(KeySource::Passkey));
        assert_eq!(
            fs::read_to_string(&sidecar).unwrap(),
            half,
            "left for the sync to finish"
        );

        // The plugin's abandoned enrollment: the file wraps a key the database never got.
        fs::write(envelope::legacy_old_path_for(&db), &current).unwrap();
        fs::write(&sidecar, ring_for(&k2, &[1, 2])).unwrap();
        drop(unlock_with(KeySource::Passkey));
        assert_eq!(fs::read_to_string(&sidecar).unwrap(), current, "repaired");
        clear_and_remove(&dir);
    }

    /// A password-only database with one entry and a fast KDF.
    fn small_db(path: &Path) {
        use vault_core::keepass::{Database, DatabaseKey, config::KdfConfig, db::fields};
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
        db.root_mut()
            .add_entry()
            .set_unprotected(fields::TITLE, "Bank");
        db.save(
            &mut fs::File::create(path).unwrap(),
            DatabaseKey::new().with_password("pw"),
        )
        .unwrap();
    }

    /// The vault's backups in its config's roots, newest first.
    fn stored(vault: &Vault) -> Vec<vault_core::backups::Backup> {
        let config = vault.backup_config().expect("a store").clone();
        let (folders, _) = config.existing_folders(
            vault.path(),
            &vault_core::backups::vault_id_of(&vault.db),
            &mut Default::default(),
        );
        folders.iter().flat_map(|f| f.list().unwrap()).collect()
    }

    fn opened_copies(vault: &Vault) -> usize {
        stored(vault)
            .iter()
            .filter(|b| b.name.kind() == Some(vault_core::backups::Kind::Opened))
            .count()
    }

    #[test]
    fn unlock_snapshot_dedupes_by_hash() {
        let dir = temp_dir();
        let db = dir.join("synthetic.kdbx");
        small_db(&db);
        let mut first = unlocked(unlock_password(&db, KdfBudget::DEFAULT));
        assert_eq!(opened_copies(&first), 1);
        assert!(first.take_backup_notice().is_none());
        drop(first);
        // Unchanged: the newest copy already holds these bytes.
        let again = unlocked(unlock_password(&db, KdfBudget::DEFAULT));
        assert_eq!(opened_copies(&again), 1);
        drop(again);

        // Another app saves a new version: the next unlock keeps it too.
        {
            use vault_core::keepass::{Database, DatabaseKey, db::fields};
            let key = || DatabaseKey::new().with_password("pw");
            let mut other = Database::parse(&fs::read(&db).unwrap(), key()).unwrap();
            other
                .root_mut()
                .add_entry()
                .set_unprotected(fields::TITLE, "Added elsewhere");
            let mut bytes = Vec::new();
            other.save(&mut bytes, key()).unwrap();
            fs::write(&db, bytes).unwrap();
        }
        let after = unlocked(unlock_password(&db, KdfBudget::DEFAULT));
        let copies = stored(&after);
        assert_eq!(opened_copies(&after), 2);
        assert_eq!(
            copies[0].sha256.as_deref(),
            Some(&*vault_core::backups::sha256_hex(&fs::read(&db).unwrap()))
        );
        assert_eq!(copies[0].generation.as_deref(), Some(after.generation()));
        drop(after);
        clear_and_remove(&dir);
    }

    /// A PC that mostly unlocks (another device saves) never saves here, so unlock runs the
    /// clean-up a save would: a backlog of copies shrinks (a quarter at a time), and ring
    /// snapshots from before a key change made elsewhere go once the current ring is kept.
    #[test]
    fn unlock_cleans_up_a_backlog_and_old_rings() {
        use vault_core::backups::{Kind, WriteOutcome};
        let dir = temp_dir();
        let db = dir.join("synthetic.kdbx");
        small_db(&db);
        // A save records the credentials in the file, so the next unlock's generation is
        // committed.
        let mut first = unlocked(unlock_password(&db, KdfBudget::DEFAULT));
        first.mark_dirty();
        first.save().unwrap();
        let config = first.backup_config().unwrap().clone();
        let folder = config
            .open_folder(
                &config.home(),
                &db,
                &vault_core::backups::vault_id_of(&first.db),
                &mut Default::default(),
                false,
            )
            .unwrap()
            .unwrap();
        drop(first);
        let mut bytes = fs::read(&db).unwrap();
        for i in 0..40u32 {
            bytes.extend(i.to_le_bytes());
            let written = folder
                .write_backup(&bytes, Kind::Opened, None, None)
                .unwrap();
            assert!(matches!(written, WriteOutcome::Written(_)));
        }
        let old_ring = folder
            .write_ring_snapshot(b"a ring from before a key change", Some("replaced"))
            .unwrap();
        fs::write(
            envelope::path_for(&db),
            KeyRing {
                keys: vec![RingKey {
                    label: "Phone passkey".into(),
                    envelope: Envelope::seal(RP_ID, &[1], [2; 32], &[3; 32], &[4; 32]),
                }],
            }
            .serialize(),
        )
        .unwrap();
        let before = folder.list().unwrap().len();

        let vault = unlocked(unlock_password(&db, KdfBudget::DEFAULT));
        // Its own `opened` copy, then at most a quarter of them gone.
        let after = folder.list().unwrap().len();
        assert!(
            after < before && after + (before + 1) / 4 > before,
            "{before} then {after}"
        );
        assert!(!old_ring.path().exists());
        let rings = folder.list_rings().unwrap();
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].generation.as_deref(), Some(vault.generation()));
        drop(vault);
        clear_and_remove(&dir);
    }

    #[test]
    fn recovered_session_still_uses_store() {
        let dir = temp_dir();
        let db = dir.join("synthetic.kdbx");
        small_db(&db);
        // A session locked with unsaved changes, held encrypted in memory.
        let mut session = Vault::open(
            &db,
            Credentials {
                password: Some(Zeroizing::new("pw".into())),
                key_file: None,
            },
        )
        .unwrap();
        session.mark_dirty();
        let sealed = session.seal().unwrap();
        drop(session);

        let mut recovered = unlocked(unlock_with(&db, Some(sealed), KdfBudget::DEFAULT));
        assert!(recovered.is_dirty() && !recovered.is_read_only());
        assert!(
            stored(&recovered).is_empty(),
            "no `opened` copy for a recovery"
        );
        let previous = fs::read(&db).unwrap();
        recovered.save().unwrap();
        let copies = stored(&recovered);
        assert_eq!(copies.len(), 1);
        assert_eq!(copies[0].name.kind(), Some(vault_core::backups::Kind::Auto));
        assert_eq!(fs::read(&copies[0].path).unwrap(), previous);
        assert!(!vault_core::vault::with_suffix(&db, ".bak").exists());
        drop(recovered);
        clear_and_remove(&dir);
    }

    /// A backup in a vault's folder under a backup root opens read-only and gets no store.
    /// Another vault elsewhere under the root (a USB drive chosen as the backup folder) is a
    /// vault like any other.
    #[test]
    fn a_backup_opened_as_a_vault_is_read_only() {
        let dir = temp_dir();
        let root = dir.join("Backups");
        let inside = root.join("synthetic");
        fs::create_dir_all(&inside).unwrap();
        fs::write(
            inside.join(vault_core::backups::VAULT_FILE),
            br#"{"v":1,"id":"0123456789abcdef0123456789abcdef","stem":"synthetic"}"#,
        )
        .unwrap();
        let backup = inside.join("synthetic 2026-10-14 09-00-00 #000001.kdbx");
        small_db(&backup);
        let before = fs::read(&backup).unwrap();
        let open = |path: &Path| {
            unlocked(unlock(
                path.to_owned(),
                None,
                Some(Zeroizing::new("pw".into())),
                KeySource::None,
                WindowHandle(0),
                Default::default(),
                KdfBudget::DEFAULT,
                BackupConfig::new(&root),
            ))
        };
        let mut vault = open(&backup);
        assert!(vault.is_read_only() && vault.backup_config().is_none());
        vault.mark_dirty();
        assert!(matches!(vault.save(), Err(VaultError::ReadOnly)));
        assert_eq!(fs::read(&backup).unwrap(), before);
        assert_eq!(
            fs::read_dir(&inside).unwrap().count(),
            2,
            "nothing written beside it and vault.json"
        );
        drop(vault);

        let portable = root.join("Portable").join("Vault.kdbx");
        fs::create_dir_all(portable.parent().unwrap()).unwrap();
        small_db(&portable);
        let mut vault = open(&portable);
        assert!(!vault.is_read_only() && vault.backup_config().is_some());
        vault.mark_dirty();
        vault.save().unwrap();
        drop(vault);
        clear_and_remove(&dir);
    }

    /// Backups are read-only; clear that before removing the test folder.
    fn clear_and_remove(dir: &Path) {
        fn clear(dir: &Path) {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    clear(&path);
                } else {
                    let _ = vault_core::fsops::set_readonly(&path, false);
                }
            }
        }
        clear(dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn oversized_key_file_is_refused() {
        let dir = temp_dir();
        let key_file = dir.join("huge.key");
        // Sized without writing: the length check refuses it before any byte is read.
        fs::File::create(&key_file)
            .unwrap()
            .set_len(limits::KEY_FILE_MAX + 1)
            .unwrap();
        let msg = failure(unlock(
            dir.join("synthetic.kdbx"),
            None,
            Some(Zeroizing::new("pw".into())),
            KeySource::KeyFile(key_file),
            WindowHandle(0),
            Default::default(),
            KdfBudget::DEFAULT,
            backups_beside(&dir.join("synthetic.kdbx")),
        ));
        assert!(msg.contains("larger than 16 MiB"), "{msg}");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn heavy_kdf_is_reported_and_opens_once_approved() {
        use vault_core::keepass::{Database, DatabaseKey, config::KdfConfig};
        let dir = temp_dir();
        let db = dir.join("synthetic.kdbx");
        let mut database = Database::new();
        // Argon2's 101 passes are over the budget, yet over 64 KiB they are quick.
        if let KdfConfig::Argon2 {
            iterations,
            memory,
            parallelism,
            ..
        } = &mut database.config.kdf_config
        {
            (*iterations, *memory, *parallelism) = (101, 64 * 1024, 1);
        }
        database
            .save(
                &mut fs::File::create(&db).unwrap(),
                DatabaseKey::new().with_password("pw"),
            )
            .unwrap();

        let Err(UnlockError::HeavyKdf(kdf)) = unlock_password(&db, KdfBudget::DEFAULT) else {
            panic!("an over-budget database opened without approval");
        };
        assert_eq!(kdf.details(), "Argon2d · 0 MB × 101 · 1 lane");
        let msg = UnlockError::HeavyKdf(kdf.clone()).into_message();
        assert!(msg.contains("Unlock anyway only if"), "{msg}");
        assert!(unlock_password(&db, KdfBudget::DEFAULT.allowing(&kdf)).is_ok());
        clear_and_remove(&dir);
    }

    #[test]
    fn oversized_passkey_file_is_refused_before_any_prompt() {
        let dir = temp_dir();
        let db = dir.join("synthetic.kdbx");
        let ring = KeyRing {
            keys: vec![RingKey {
                label: "Phone passkey".into(),
                envelope: Envelope::seal(RP_ID, &[1], [2; 32], &[3; 32], &[4; 32]),
            }],
        }
        .serialize();
        let asked = Cell::new(false);
        let evaluate = |_: &str, _: &[PrfCredential]| -> Result<PrfAnswer, PasskeyError> {
            asked.set(true);
            Ok((0, Zeroizing::new([3; 32])))
        };

        // Blank lines are ignored, so a ring padded to exactly the cap still unlocks.
        let padded = format!(
            "{ring}{}",
            "\n".repeat(limits::SIDECAR_MAX as usize - ring.len())
        );
        fs::write(envelope::path_for(&db), &padded).unwrap();
        let Ok(key) = passkey_key(&db, evaluate) else {
            panic!("a ring at the cap failed to unlock");
        };
        assert_eq!(&key[..], &[4; 32]);
        assert!(asked.replace(false));

        fs::write(envelope::path_for(&db), format!("{padded}\n")).unwrap();
        let msg = failure(passkey_key(&db, evaluate));
        assert!(msg.contains("larger than 64 KiB"), "{msg}");
        assert!(!asked.get(), "no passkey prompt for an oversized file");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn passkey_unlock_offers_only_this_apps_keys() {
        let dir = temp_dir();
        let db = dir.join("synthetic.kdbx");
        let key = [4u8; 32];
        // The foreign key comes first: the old code took the relying party from keys[0].
        let foreign = RingKey {
            label: "Other site".into(),
            envelope: Envelope::seal("example.com", &[9], [2; 32], &[9; 32], &[8; 32]),
        };
        let ours = RingKey {
            label: "Phone passkey".into(),
            envelope: Envelope::seal(RP_ID, &[1], [2; 32], &[1; 32], &key),
        };
        let write = |keys: Vec<RingKey>| {
            fs::write(envelope::path_for(&db), KeyRing { keys }.serialize()).unwrap();
        };

        write(vec![foreign.clone(), ours]);
        let unlocked = passkey_key(&db, |rp_id, creds| {
            assert_eq!(rp_id, RP_ID);
            let ids: Vec<&[u8]> = creds.iter().map(|c| c.credential_id).collect();
            assert_eq!(ids, [&[1u8][..]], "only this app's key is offered");
            Ok((0, Zeroizing::new([1; 32])))
        });
        let Ok(unwrapped) = unlocked else {
            panic!("this app's key failed to unlock");
        };
        assert_eq!(&unwrapped[..], &key);

        // An index outside the offered list is a failure, not a panic.
        let msg = failure(passkey_key(&db, |_, _| Ok((1, Zeroizing::new([9; 32])))));
        assert!(msg.contains("does not unlock"), "{msg}");

        write(vec![foreign]);
        let asked = Cell::new(false);
        let msg = failure(passkey_key(&db, |_, _| {
            asked.set(true);
            Ok((0, Zeroizing::new([9; 32])))
        }));
        assert!(msg.contains("no key for Axentra Vault"), "{msg}");
        assert!(!asked.get(), "another site's passkeys are never asked");
        fs::remove_dir_all(dir).ok();
    }
}
