# Axentra Vault

A Windows password manager for standard KeePass (`.kdbx`) files, written in Rust. It can be unlocked with the master
password plus a phone passkey (iPhone, Android) or a FIDO2 security key (YubiKey), using the WebAuthn **PRF**
extension. The file stays plain KDBX 4, so KeePass, KeePassXC and phone KeePass apps can still open it with the
master password and the backup key file.

Status: pre-release. The external security audit is in [`docs/AUDIT_REPORT.md`](docs/AUDIT_REPORT.md) (brief:
[`docs/AUDIT_BRIEF.md`](docs/AUDIT_BRIEF.md)). Fixes are on the `audit/fixes` branch.

## Features

- **Unlock keys:** add several passkeys or security keys; any one of them unlocks the vault. Adding a key reuses the
  database key, so there's no re-key and the backup `.keyx` stays valid. Removing a key rotates the database key,
  re-wraps it for every key you keep, and writes a new backup key file, so the removed key really is revoked.
- **KeePass fidelity:** history (with KeePass's oldest-first order), attachments, custom and protected fields,
  tags, icons and auto-type are kept. Round trips are checked against KeePassLib (`interop/`).
- **Organization:**
  - ranked search; email-domain-only matches are hidden until asked for
  - smart categories, with a manual override
  - Archive ("tombstone") with full history kept
  - previous passwords
  - favorites and tags
- **Watchtower:** flags reused, weak and old passwords, plus an **opt-in breached-password check**
  (Have I Been Pwned, k-anonymity: see below).
- **Layouts:** Floating Modular (frameless, transparent) and Classic. In Classic you choose and sort the table
  columns, including custom fields.
- **Site icons (optional):** each site's own favicon, fetched over HTTPS from public addresses only and stored as a
  KeePass custom icon.

## Security design

**Keys.** The database key is master password + a 32-byte key file. With passkey unlock, that 32-byte key is
wrapped in `<db>.axvault`:
- HKDF-SHA256 (salt = PRF input) derives the keys from the PRF output.
- The 32-byte key is encrypted with AES-256-CBC + HMAC-SHA256, encrypt-then-MAC, with every field length-prefixed.
- The PRF output is returned to this PC by Windows during unlock and kept in memory only. It is never stored.
- The same 32 bytes are saved once as the backup key file (`.keyx`, KeePass key-file format 2.0).
- A vault that already uses a key file keeps it. The passkey wraps the 32 bytes KeePass derives from that file
  (XML key data, a 32-byte or 64-hex-digit file as is, otherwise the file's SHA-256), and only after checking that
  they open the database. An XML key file whose data isn't 32 bytes gets a new key and backup key file instead.
- `.axvault` holds one key (`AxentraVault/1`) or several (`AxentraVault/2`).

**Against other programs on the PC, running as the same user (no admin):**
- **Process access list:** other programs can't read or write the vault's memory, create threads in it, or
  duplicate its handles. Only SYSTEM keeps full access, and an OWNER RIGHTS entry stops the owner re-granting itself
  access. This is the same approach KeePassXC uses.
- **Mitigation policies:**
  - legacy extension points are off: AppInit DLLs, Winsock providers, global hook DLLs
  - no DLLs from network shares or low-integrity locations
  - System32 copies are preferred
- **Screen capture:** windows are excluded from screenshots, recording and screen sharing. This is a setting, on by
  default.
- **Clipboard:** kept out of clipboard history and cloud clipboard, and cleared after 12 s.
- **Secret fields:** they keep no undo history, and locking clears all widget state.
- **Auto-lock:** after 5 minutes. If saving fails it still locks and holds the unsaved changes as encrypted KDBX in
  memory.

These defences can't stop malware running as administrator or SYSTEM.

**Windows security status.** Settings shows the antivirus health (Windows Security Center) and the Smart App
Control state. You're warned when antivirus needs attention. These are local checks.

**Network.** The app goes online only when you ask:
- **Site icons:** sent to each site, HTTPS only. Requests can only reach public addresses, and redirects are
  followed only within the same site.
- **Breach check:** sent to `api.pwnedpasswords.com`. Each password is SHA-1 hashed locally and only the first 5 hex
  characters are sent, with response padding on. Matching happens locally, and results stay in memory for the
  session.

## Layout

| Path | What |
|---|---|
| `crates/vault-core/` | KDBX open/save with verification, the versioned backup store (`backups.rs`, `store.rs`, `manage.rs`) and its re-encryption on key changes (`rekey.rs`), envelope and key ring, key files, search, organization, categories, header (KDF) reading |
| `crates/passkey-win/` | `webauthn.dll` FFI: register (PRF + `hmac-secret`), PRF evaluation over one or many credentials |
| `app/` | eframe/egui app: `session.rs` (Classic), `session/floating.rs` (Floating), `session/backups.rs` (Settings > Backups), `hardening.rs`, `breach.rs`, `siteicons.rs`, `secret_edit.rs`, `jobs.rs` |
| `interop/` | KeePassLib ⇄ Rust round-trip tests |
| `keepass-plugin/` | Legacy KeePass 2.x key-provider plugin using the same `.axvault` format |
| `docs/` | Audit brief and report, design handoff, logo spec |

## Build and test

```powershell
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
.\interop\run.ps1          # needs KeePass 2.x installed
.\scripts\deploy-test.ps1  # copy the debug build to %LOCALAPPDATA%\Programs and add a desktop shortcut
```

Smart App Control blocks unsigned release builds; releases need code signing.

## Requirements and limits

- Windows 10/11, 64-bit. Passkey unlock needs Windows WebAuthn API 6 or later (current Windows 11). Phone passkeys
  need Bluetooth.
- FIDO2 security keys need `hmac-secret` (YubiKey 5 series, Security Key series) and a FIDO2 PIN.
- Passkeys use relying-party ID `localhost`, and show on the phone as "localhost".
- iCloud Keychain passkeys sync through your Apple Account, so that account's security matters.
- Phone KeePass apps use the backup key file, not the passkey.

## Backups

Before a save replaces the database, the exact previous version is copied into a backup folder for that vault. No
`.bak` is written beside the database, where it would sync along with it.
- **Where:** `%LOCALAPPDATA%\Axentra Vault\Backups` by default: this PC only, private to your Windows account, not
  synced. Settings > Backups > Change… picks another folder, such as a USB drive. Axentra marks it with an
  `.axentra-backups` file and never creates it. It warns about OneDrive, Dropbox and Google Drive folders, FAT and
  exFAT drives (no file permissions), network folders and folders that already hold KeePass files, and refuses the
  vault's own folder. Earlier folders stay known (their backups are still listed, re-encrypted on key changes and
  cleaned up) until you forget them in Settings > Backups. Each vault, and each copy of it, keeps its own folder in
  every backup folder.
- **When the folder is unavailable:** the default folder takes the backup. If that fails too, an ordinary save keeps
  `<db>.bak` beside the database until the backup folder works again, then moves it in. A save that changes the
  credentials never falls back to `<db>.bak`, and with no backup at all nothing is saved.
- **Also kept:** the version found at unlock whenever it differs from the newest backup (another app's save, or the
  version Axentra saved last, which otherwise exists only in the vault file), "Back up now" copies, the version a
  restore or "Save anyway" replaced (also a file that can't be read any more, after you confirm), and old `<db>.bak`
  and `<db>.tmp` files. After unlock Axentra offers to move these into the backup folder (copied and checked first),
  with "Don't ask again".
- **Files:** `<vault> YYYY-MM-DD HH-MM-SS #<number>[ kind].kdbx`, named after the version's own time, never replaced
  and read-only, so other apps can't save over them. The number orders them, not the clock.
- **Retention:** the newest 20 saves, one per day for the last 30 days with saves, one per month for the last 12
  months with saves, and every manual, restore, key-change and imported backup. Clean-up runs after each save and
  unlock and deletes at most a quarter of the backups at a time, oldest first.
- **Restore:** Settings > Backups > Restore… opens the backup with your current credentials and saves it as the
  vault with your current key, key derivation settings and passkeys, keeping the replaced version. It is never a file
  copy: copying a backup over your vault can lock you out. Backups that need older credentials can't be restored
  yet.
- **Opening a backup file directly:** it opens read-only, with your master password + backup key file (passkeys
  unlock only the main vault). Changes aren't saved and it isn't remembered as the last vault.
- **Deleting:** Settings > Backups deletes the backups that need older credentials (those from before a key change,
  made in Axentra or, as Axentra tells you at unlock, in another app), or all of them.
- Backups open with the same credentials as the vault and don't replace the backup key file. Uninstalling keeps
  them; delete the folder yourself if you no longer want them.

## Credential changes and older copies

Setting up passkey unlock or revoking a key changes the database key. Before the new key is saved, every backup in
each backup folder Axentra can reach (and every copy of the database beside it, such as `<db>.bak` and the `.tmp`
files earlier versions or an interrupted save left, which move into the backup folder) is re-encrypted to it, and
the old-key versions are then deleted. One that another program holds open is deleted at the next unlock. A backup
that can't be re-encrypted without losing something (written by another app, an older KDBX version, or unusually
heavy key derivation) keeps its old credentials and is marked. Old passkey files (`.axvault.old`, ring snapshots of
earlier keys, on every PC at its next unlock) are deleted. Cancelling deletes the new backup key file again. Copies
outside the backup folder, including OneDrive version history, other apps' backups and phones, can't be changed, so
review them separately after adding a second factor or revoking a key.

## License

GPL-3.0-or-later.
