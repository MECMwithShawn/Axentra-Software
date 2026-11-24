# Axentra Vault security audit brief

You are auditing **Axentra Vault** before its first packaged release (signed exe + MSI). It is a
Windows desktop password manager written in Rust (eframe/egui 0.36). It reads and writes standard
KeePass **KDBX 4** files. Its extra feature is unlocking with a phone passkey or a FIDO2 security key
(YubiKey) through the WebAuthn **PRF** extension.

One person uses it daily for their real password database (about 300 entries: banking,
government, work). Please assume real money and identity are at stake.

**Deliverable:** a report at `docs/AUDIT_REPORT.md`, using the format at the end of this brief.
You may also propose fixes as small, separate commits on a branch named `audit/fixes`. One finding
per commit. Each fix needs a test where practical. Do not rewrite or restyle working code. Do not
change the on-disk formats unless a finding requires it, and say so explicitly when one does.

The baseline is commit `a264271` on `main`.

---

## 1. Build and test

```powershell
cargo build                                   # debug build; the release exe is blocked by Smart App Control until signed
cargo test --workspace                        # ~90 tests incl. headless UI tests (egui_kittest)
cargo clippy --workspace --all-targets        # expected: 0 warnings
.\interop\run.ps1                             # KeePassLib <-> Rust round trip (needs KeePass 2.x installed)
.\keepass-plugin\build.ps1                    # C# plugin + its tests (legacy; see below)
```

Toolchain: Rust 1.99 MSVC on Windows 11, with WebAuthn API version 9.

## 2. Architecture and file map

| Area | Files | Notes |
|---|---|---|
| KDBX open/save | `crates/vault-core/src/vault.rs` | `keepass` crate 0.15.2. Save path: serialize → re-open and verify in memory → write `.tmp` + fsync → copy the old file to `.bak` → rename. Detects external changes (`ChangedOnDisk`). |
| Passkey envelope | `crates/vault-core/src/envelope.rs` | Wraps the 32-byte database key. HKDF-SHA256(ikm = PRF output, salt = PRF input) → AES-256-CBC + HMAC-SHA256 (encrypt-then-MAC, length-prefixed fields). `KeyRing` = several keys in one `.axvault` file (`AxentraVault/1` single, `AxentraVault/2` multi). |
| Key files | `crates/vault-core/src/keyfile.rs` | Writes the KeePass XML key file v2.0 (`.keyx`) backup. `raw_key()` recovers the 32-byte key from direct-format key files. |
| WebAuthn FFI | `crates/passkey-win/src/lib.rs` | Hand-written `#[repr(C)]` structs for `webauthn.dll` (raw-dylib). MakeCredential (PRF + `hmac-secret`, exclude list) and GetAssertion with global or per-credential PRF salts. **Unsafe code: please review closely.** |
| Unlock / setup jobs | `app/src/jobs.rs` | Runs on a worker thread. Passkey unlock offers every key in the ring. Registration: first prompt registers, second prompt evaluates PRF. |
| Passkey setup / add key | `app/src/session.rs` (`start_passkey_setup`, `finish_passkey_setup`, `finish_add_key`, `remove_unlock_key`, `write_ring`) | First setup: save backup `.keyx` → re-key and save the database → write the `.axvault`. Adding a key: wrap the existing raw key, no re-key. |
| Header parsing | `crates/vault-core/src/header.rs` | Parses the unencrypted KDBX header (KDF parameters) for display. **Untrusted input.** |
| Clipboard | `app/src/clipboard.rs` | Excluded from clipboard history and cloud clipboard, cleared after 12 s. |
| Site icons | `app/src/siteicons.rs` | Optional, user-triggered. Fetches the site's own favicon (host only, no third-party service, skips IPs and LAN). Size caps, image decode limits. **Network + untrusted images.** |
| Launch URL | `app/src/session/floating.rs` (`launch_url`) | `ShellExecuteW`, http(s) only. |
| UI / state | `app/src/app.rs`, `session.rs`, `session/floating.rs` | Auto-lock after 5 min idle. Locked screen must show no vault data. Editor runs in a separate OS window (immediate viewport). |
| Settings | `app/src/settings.rs`, `app/src/columns.rs` | `%APPDATA%\Axentra Vault\*.txt`. Must hold **no vault contents** (only paths, layout, column names, sort, unlock timings). |
| Legacy KeePass plugin | `keepass-plugin/` (C#) | The user no longer uses KeePass. Lower priority, but it shares the envelope format, so format bugs matter. |

## 3. Threat model

**Assets:** entry passwords and protected fields, the master password, the 32-byte database key,
the PRF output, and the backup `.keyx`.

**In scope:**
- **Stolen or synced files.** An attacker has the `.kdbx`, the `.axvault` and possibly the `.bak`/`.old` files (OneDrive sync). They must not be able to open the database without master password + (passkey **or** backup key file).
- **Malicious files.** A crafted `.kdbx`, `.axvault`, key file, or KDBX header opened by the user. Panics, unbounded allocation, or logic errors that weaken unlock.
- **Malicious websites** during the optional icon fetch. Redirects, huge responses, decompression or image bombs, SSRF to LAN/localhost, a non-image payload.
- **Same-user, non-admin malware** running after the vault is locked or closed. Secrets left in memory, the clipboard, temp files, or settings.
- **Data loss.** Crash or power loss mid-save, a failure partway through passkey setup, and concurrent edits from another KeePass app.
- **Correctness of FFI.** Struct layouts, pointer lifetimes across the `webauthn.dll` calls, and freeing returned structs.

**Out of scope:** an attacker with admin/kernel access while the vault is unlocked, a compromised
Windows WebAuthn stack, physical attacks on the phone or YubiKey, and the `keepass` crate's
own cipher implementations (but please flag misuse of them).

## 4. Specific questions

1. **Envelope crypto:**
   - Is HKDF(salt = PRF input) plus AES-CBC + HMAC (encrypt-then-MAC, MAC over magic, rpId, credentialId, prfInput, iv, ciphertext) sound?
   - Is the MAC comparison constant-time?
   - Is padding-oracle exposure impossible, since the MAC is verified first?
   - Note: the label lines in the multi-key file are **not** MAC'd. Is that acceptable? An attacker can rename labels.
2. **Key ring:**
   - All keys share one PRF input when a key is added. Any weakness from reusing the PRF salt across different credentials?
   - Can an attacker who can write the `.axvault` file cause harm beyond denial of service? For example, by swapping in their own key, causing a downgrade, or confusing which credential is used.
3. **First-setup ordering** (`finish_passkey_setup`): is there any failure point that leaves the user locked out (no working key file and no envelope)?
4. **Memory hygiene:**
   - Where do plaintext secrets get copied without `Zeroizing`? Look at the egui `TextEdit` buffers, the `String`s from the `keepass` crate, `Row`/display caches, and the clipboard path.
   - What survives a lock? Is everything from `Session` dropped?
5. **FFI** (`passkey-win`):
   - Struct sizes and versions vs `webauthn.h`. Is `HmacSecretSaltValues.cred_salts` correct?
   - Are the lifetimes of `Vec`s behind raw pointers valid for the whole call?
   - Is `WebAuthNFree*` always called, including on error paths?
   - Is `hwnd` handling safe?
6. **Save path:**
   - Atomicity on NTFS (rename-over-existing).
   - The `.bak` copy happens *before* the rename. Any window where both the original and `.bak` are bad?
   - Is `sync_all` enough, or is a directory flush needed?
7. **Site icons:**
   - Redirects to private IPs or localhost after DNS resolution (DNS rebinding).
   - IDN and punycode hosts.
   - Response size enforcement with chunked or compressed bodies.
   - `image` crate limits.
   - Anything that could leak entry data (paths, usernames) in requests.
8. **Untrusted parsing:** `header.rs` (index math, huge length fields), `KeyRing::parse`, `keyfile::raw_key`. Fuzzing suggestions are welcome.
9. **Auto-lock and locked UI:**
   - Can any vault data render or stay reachable while locked? Consider the separate editor viewport, the activity log, and the toast.
   - Does idle auto-lock fire during a modal dialog (rfd file dialog, passkey prompt)?
10. **Dependencies:** run `cargo audit` and `cargo deny` (or equivalent). Note anything unmaintained or with advisories, especially `ureq`, `image`, `resvg`, `keepass`, and `zeroize` usage.
11. **Packaging readiness:** anything that must change before signing and shipping an MSI? For example, debug-only code paths, test hooks reachable in release, or hard-coded paths.

## 5. Constraints

- **Keep KDBX compatibility.** The master password + backup `.keyx` must keep opening the database in KeePass and KeePassXC. Don't propose custom ciphers or formats.
- **No new network calls.** Don't add network calls or telemetry. The app only goes online when the user asks it to fetch site icons.
- **Don't use real data.** Never use the owner's real database. Test databases come from `crates/vault-core/examples/make_demo.rs` or the `ui_tests.rs` helpers.
- **Don't delete data, and don't modify anything outside the repo.**

## 6. Report format (`docs/AUDIT_REPORT.md`)

For each finding:

```
### [SEVERITY] Short title
- Where: file:line (and function)
- What: one-paragraph description
- Exploit / failure scenario: concrete steps or inputs
- Fix: recommended change (and commit hash on audit/fixes if you made one)
- Confidence: confirmed (reproduced/tested) | likely | speculative
```

Use **Critical / High / Medium / Low / Info** as the severity. Order findings most severe first. End the
report with three sections:
- "Looks good": areas you checked and found sound, so we know what was covered.
- "Not reviewed": anything you didn't get to.
- A short release-readiness verdict: ship / ship after fixes / don't ship.
