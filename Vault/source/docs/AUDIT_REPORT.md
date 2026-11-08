# Axentra Vault security audit

Audit date: 2026-10-13 (America/Chicago). Baseline: `a264271`. Reviewed checkout: `722c40ae0f9a0403ec40d703a9749275bf032cbf`; the only change from the baseline was the audit brief. Findings and line numbers below describe that baseline. This is a source review with bounded reproductions, not a certification.

**Baseline verdict: don't ship.** Six high-severity findings were reproduced. In particular, the real application's locked password field restores the previous master password with Ctrl+Z. Passing the existing suite does not establish that locking removes secrets or that passkey removal revokes access.

No owner database, real credentials, real authenticator, production settings, or external website was used in reproductions. Test data and generated outputs were kept inside this repository. Network tests used loopback listeners; public documentation and the public RustSec database were fetched for review. Application source was unchanged during baseline evidence gathering. The six high-severity remediation commits and their validation are recorded below; remaining findings retain their baseline descriptions and locations.

## Validation and evidence

- Rust 1.99.0 MSVC / Cargo 1.99.0. `cargo build --offline --locked`: passed.
- `cargo test --offline --locked --workspace`: 86 substantive unit/UI tests passed, two intentionally ignored tests (live website fetch and preview rendering). The two interoperability wrappers also returned success but skip their bodies without `AXV_INTEROP_DIR`; they were subsequently exercised with real synthetic fixtures.
- `cargo clippy --offline --locked --workspace --all-targets`: passed, no warnings.
- KeePass interop: compiled the repository's C# fixture driver against the existing repo-local KeePass 2.61.1 library copy; generated fresh fixtures in `target/audit/interop-df18e32606ac4cd08d48b6eb0377dd96/fixtures`; both Rust interoperability tests and all 37 C# verification assertions passed. Four redundant fallback icons were reported dropped by the existing fidelity check; custom icon contents survived.
- `keepass-plugin/build.ps1 -KeePassExe interop/out/KeePass.exe`: built successfully; all nine plugin tests passed. The default installed KeePass path was absent. The interop script's delete-and-recreate step was replaced with a fresh fixture directory, preserving existing files.
- [Reproduction runner](audit-evidence/reproduce.ps1): 14 bounded tests passed. These assert the presence of baseline defects, so they are evidence tests, not a release regression suite. The runner now archives the pinned reviewed revision, including all local crates, into a unique harness under `target/audit`; it does not revert or alter product source. It invalidates generated local-package Cargo artifacts to prevent baseline/fixed cache contamination. [Output](audit-evidence/reproduction-results.txt).
- `cargo audit` and `cargo deny` are not installed. Equivalent advisory matching used all 536 lockfile packages, public RustSec snapshot `6aed84668c23d7f535077a9fe918bf1fa0f6a5bc`, and Rust's `semver` implementation for patched/unaffected ranges. Withdrawn advisories were excluded. Windows normal/build dependency reachability, declared licenses, duplicate versions, sources and cached-index yank flags were inventoried. [Results](audit-evidence/dependency-results.json). This is not a successful run of either absent tool, nor a complete cargo-deny policy evaluation; there is no project `deny.toml`. Yank data is cached, not freshly verified.
- FFI was compared against the installed Windows SDK 10.0.26100.0 `webauthn.h` and [Microsoft's header](https://github.com/microsoft/webauthn/blob/master/webauthn.h). Existing x64 size and API-availability tests passed. No physical passkey ceremony was performed.

## Findings

### [High] H1 — Master password and entry passwords survive locking in egui undo history
- Where: `app/src/app.rs:233,399` (screen replacement, unlock TextEdit); `app/src/session.rs:1635,1714` (editor); `crates/vault-core/src/form.rs:19,110`.
- What: Dropping the session and its Zeroizing buffers does not clear the persistent egui Context. egui 0.36.2 records real text in TextEdit undo state even in password mode. The locked form reuses its widget identity. The baseline's assertion that dropping the screen zeroizes the database and typed password is therefore insufficient. Other ordinary plaintext copies include protected notes in EntryForm, protected custom values in custom_map, display strings, and label/layout caches. KeePass protected values use SecretBox for zeroization, not encryption of live memory.
- Exploit / failure scenario: Unlock a synthetic vault, click Lock, focus the now-empty master password field, press Ctrl+Z: the previous master password reappears in the underlying field. This was reproduced through the actual VaultApp UI. A second test retrieves an entry-password canary from TextEdit state after Session is dropped. Same-user malware attaching after lock can inspect these live allocations.
- Fix: Prevent secret undo snapshots from being created; purge UI state at security transitions; scrub application-owned plaintext copies and do not depend on password masking for memory protection. Add real unlock/lock/undo and context-lifetime regression tests. Audit framework caches and worker-owned secrets explicitly.
- Remediation: implemented on `audit/fixes` in `81ed196`. See the remediation verification and limits below.
- Confidence: confirmed (reproduced/tested).

### [High] H2 — Auto-lock silently fails open when saving fails
- Where: `app/src/session.rs:423` (`frame_logic`).
- What: An overdue dirty vault locks only when save succeeds. Any error resets the activity timer without showing an error, retaining the unlocked vault for another five minutes. Repeated failures keep it unlocked indefinitely.
- Exploit / failure scenario: Edit a vault, change the file externally, then leave the app unattended. Once overdue, ChangedOnDisk prevents lock and resets the timer. A synthetic reproduction verified SessionAction::None, a refreshed timer, and no error. Disk-full, permission, sharing and network-drive errors follow the same branch.
- Fix: Make the lock deadline independent of persistence success. Preserve unsaved data in an encrypted recovery record or sealed in-memory session, then remove all plaintext from the live UI. Present the save conflict after reauthentication; never silently extend the deadline.
- Remediation: implemented on `audit/fixes` in `4097ea9`. See the remediation verification and limits below.
- Confidence: confirmed (reproduced/tested).

### [High] H3 — First passkey setup creates a password-only backup
- Where: `crates/vault-core/src/vault.rs:143` (`save_overwriting`); `app/src/session.rs:2154` (`finish_passkey_setup`).
- What: Re-keying the main database copies its pre-setup version verbatim to .bak. For a password-only vault, this newly created adjacent backup still opens with only the master password. Copying or syncing the directory therefore retains a weaker copy of its secrets immediately after the app advertises second-factor protection.
- Exploit / failure scenario: Re-key a synthetic password-only vault with a new 32-byte key. Password-only opening of the current database fails, but opening its .bak succeeds. An attacker with the directory can attack the master password without needing the passkey.
- Fix: Make credential changes protect the current previous-version backup under the new composite credentials before publishing the new database. Keep recovery safe across failures. Explain that historical cloud versions and copies already taken cannot be retroactively strengthened. Do not silently delete user backups.
- Remediation: implemented on `audit/fixes` in `f7e87b9`. See the remediation verification and limits below.
- Confidence: confirmed (reproduced/tested).

### [High] H4 — Removing a passkey does not revoke access to future vault contents
- Where: `app/src/session.rs:2044,2066,2189` (`remove_unlock_key`, `write_ring`).
- What: Removal edits only the sidecar list while retaining the same database key. write_ring saves the removed envelope in .old, and any earlier synced/copied envelope has the same effect. The UI's claim that the removed key no longer unlocks the vault is false for a file attacker.
- Exploit / failure scenario: Remove credential A while retaining B, save new entries, then unwrap A's envelope from .old using A's PRF output. That key plus the master password opens the latest database, including entries added after removal. The reproduction verifies those new contents. Possession of a removed authenticator alone does not bypass the master password.
- Fix: Real revocation requires a new random database key, new envelopes for retained credentials, and a matching replacement backup key file, using a recoverable transaction. If those prerequisites cannot be completed, leave revocation unapplied. Merely deleting .old cannot revoke copies already stolen.
- Remediation: implemented on `audit/fixes` in `9a6101f`. See the remediation verification and limits below.
- Confidence: confirmed (reproduced/tested).

### [High] H5 — Icon fetching can contact private services and arbitrary third-party hosts
- Where: `app/src/siteicons.rs:23,45,144,264` (`icon_host`, `agent`, `resolve`, `get_bytes`).
- What: Validation applies only to the entry's original hostname string. HTML icon hrefs and automatic redirects are unrestricted HTTP(S) URLs. DNS results are never filtered or pinned. Public-looking names can resolve to loopback/LAN, and third-party links violate the stated site-only behavior.
- Exploit / failure scenario: An allowed site's page declares an icon at an HTTP loopback endpoint, or redirects an icon request there. The exact parsing/request functions successfully fetched a synthetic loopback service in both tests. No existing local service was contacted. DNS rebinding is supported by the missing address controls, but was not separately simulated.
- Fix: Enforce an origin policy for every candidate and redirect; resolve through a public-address-only resolver and connect to those checked addresses, including IPv4-mapped IPv6 and other non-public ranges. Avoid automatic redirect behavior that bypasses policy. Reject proxy configurations that defeat local resolution guarantees or explicitly account for them.
- Remediation: implemented on `audit/fixes` in `9959b6f`. See the remediation verification and limits below.
- Confidence: confirmed (reproduced/tested); DNS-rebinding variant is source-supported.

### [High] H6 — Downloaded SVGs can open local files
- Where: `app/src/siteicons.rs:171` (`to_png`, default usvg options).
- What: The default usvg image resolver interprets image href strings as filesystem paths. The downloaded SVG is therefore allowed to read local files outside the icon request. On Windows a UNC path can also trigger filesystem network access; that authentication/network consequence was not exercised. The separate raster-image limits do not constrain SVG embedded image decoding.
- Exploit / failure scenario: Pass an SVG whose image href points to a synthetic absolute PNG path. to_png loads its pixels and returns them in the generated icon. A malicious site controls that same SVG input. This establishes local-image inclusion, not arbitrary text-file exfiltration over HTTP.
- Fix: Disable filesystem/string href resolution for untrusted SVG, reject external resources including UNC paths, and bound embedded data and rendering work. Keep trusted bundled asset rendering separate if it needs different options.
- Remediation: implemented on `audit/fixes` in `794871c`. See the remediation verification and limits below.
- Confidence: confirmed (reproduced/tested) for local file inclusion; UNC side effects are likely.

### [Medium] M1 — Compressed responses bypass the stated icon/page size caps
- Where: `app/src/siteicons.rs:264` (`get_bytes`).
- What: In the locked ureq 3.4.2 implementation the configured LimitReader sits below gzip decoding. The app collects the entire decoded body into a Vec, so MAX_PAGE/MAX_IMAGE limit compressed bytes, not the allocated output.
- Exploit / failure scenario: The local server sends 8,176 bytes of gzip data. get_bytes accepts it and allocates 8,388,608 decoded bytes despite MAX_IMAGE being 2,000,000. Larger expansion across six workers can exhaust memory or abort the process, losing unsaved changes. Plain chunked responses correctly hit the byte limit.
- Fix: Also cap the decoded reader with an explicit limit+1 read and reject overflow, or disable content decoding and require bounded identity responses. Bound total concurrent decode memory.
- Confidence: confirmed (reproduced/tested).

### [Medium] M2 — The setup recovery key and key ring are not durably committed
- Where: `app/src/session.rs:2151,2189` (`finish_passkey_setup`, `write_ring`).
- What: Backup .keyx and sidecar writes use fs::write without sync_all or a read-back check. The KDBX temporary file is flushed, so its credential change can reach storage before the promised recovery key or envelope. The chosen backup path is also not checked against database/sidecar/reserved paths. Ordinary returned write errors are handled in useful order, but that is not a power-loss transaction.
- Exploit / failure scenario: Power loss after the new KDBX is installed but before cached key-file/sidecar writes persist can leave the newest database inaccessible; an older .bak may still recover older contents. Selecting a colliding backup path can overwrite another participating file before save detects trouble.
- Fix: Use distinct paths verified by identity, durable exclusive temporary writes, verified recovery-key equivalence, atomic replacement and a restart recovery protocol. Avoid a generic POSIX directory-fsync prescription on Windows; test the actual filesystem/replace protocol and storage guarantees.
- Confidence: likely (source review; no power-cut or destructive alias test).

### [Medium] M3 — External-change checks have a save race
- Where: `crates/vault-core/src/vault.rs:81,114,122,130`.
- What: Open reads contents before obtaining their stamp. Save checks only length/mtime before potentially slow serialization and verification, then uses a shared fixed .tmp path without an exclusive writer protocol. Another writer can replace the database after the check. Missing/failed metadata also returns “unchanged.”
- Exploit / failure scenario: Two apps open the same database; B saves while A is doing its save KDF/verification. A can overwrite B without a ChangedOnDisk warning, retaining B only in .bak until the next save. Two Axentra writers may also interfere with the same .tmp.
- Fix: Capture a consistent source snapshot, use unique exclusive temp files, revalidate a content/file-identity stamp immediately before commit and coordinate writers with an appropriate Windows handle/locking protocol. Treat missing/unreadable source files as conflicts.
- Confidence: likely (source review; no concurrent-write stress/fault injection).

### [Medium] M4 — Untrusted file and KDF inputs lack resource budgets
- Where: `app/src/jobs.rs:81,97`; `crates/vault-core/src/vault.rs:80`; `crates/vault-core/src/envelope.rs:114,193`.
- What: Key files, KDBX and sidecars are read wholly without size limits; ring count, credential IDs and ciphertext are unbounded. KDBX KDF parameters are passed to keepass without a resource policy before keyed header authentication. A public header SHA-256 does not make those parameters trusted.
- Exploit / failure scenario: The ring parser accepts a one-megabyte credential ID in the bounded test. Arbitrarily larger files/rings can allocate proportionally; a crafted header can request expensive AES rounds or Argon2 memory/iterations before credentials are rejected. This audit deliberately did not exhaust host memory.
- Fix: Cap inputs and all envelope fields (ciphertext is exactly 48 bytes here); cap key counts; validate expected RP IDs and duplicate IDs; preflight KDF budgets with an explicit user-approved exceptional path. Consider an isolated cancellable parser/KDF process.
- Confidence: confirmed (reproduced/tested) for unbounded ring fields; resource exhaustion and KDF denial of service are likely.

### [Medium] M5 — raw_key can wrap a different key than KeePass uses
- Where: `crates/vault-core/src/keyfile.rs:15`; `app/src/session.rs:1986`.
- What: raw_key trims text before recognizing 64 hex digits. keepass recognizes raw hex only when the original file is exactly 64 bytes; otherwise it hashes a bare file. The separate ad-hoc XML recognition can also disagree with the actual key parser. A successful registration self-check validates the envelope, not that its key opens this database.
- Exploit / failure scenario: A 65-byte key file containing 64 hex digits plus newline opens the synthetic KDBX. raw_key returns the decoded hex bytes, but that returned key cannot open it. Add-key would report success with an unusable passkey; the original file still works.
- Fix: Derive/validate exactly the effective key KeePass used, or restrict to canonical supported forms and verify the proposed raw key against the current database before publishing an envelope.
- Confidence: confirmed (reproduced/tested).

### [Medium] M6 — Ordinary text copying bypasses clipboard protections
- Where: `app/src/session.rs:1635,1679,1714`; `app/src/clipboard.rs:34,81`; `app/src/app.rs:137,214`.
- What: Revealing a secret sets TextEdit password(false), so Ctrl+C emits egui's normal CopyText command rather than copy_secret. This bypasses exclusion markers and the 12-second PendingClear. Separately, the protected path consumes PendingClear even if OpenClipboard/EmptyClipboard fails, and locking does not immediately clear it. Marker-setting failures are ignored.
- Exploit / failure scenario: Select a revealed password/custom field and copy with Ctrl+C. A headless event test confirms the unprotected platform CopyText command. Its actual delivery to Windows history/cloud was not exercised. A busy clipboard at expiry can leave a protected-path copy without a retry.
- Fix: Route all copy/cut commands through the secret clipboard implementation, fail visibly if required exclusions cannot be set, retry failed clears, and clear on lock. Pass a valid owner HWND: Microsoft's [OpenClipboard documentation](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-openclipboard) warns that the current null-owner plus EmptyClipboard sequence can cause SetClipboardData failure. That separate native behavior needs desktop validation.
- Confidence: confirmed (reproduced/tested) for the unmanaged CopyText path; native failure/history behavior is likely and untested.

### [Medium] M7 — Modal jobs and editor drafts weaken the lock contract
- Where: `app/src/session.rs:423,1552,1854,2044,2120`; `app/src/jobs.rs:20`.
- What: Passkey work explicitly suppresses idle lock. Synchronous rfd dialogs and synchronous saves block UI progress, including lock/clipboard processing. An editor draft is not marked dirty until Save entry, so lock can drop it without the unsaved-change prompt. Worker threads have no cancellation token; dropping Job drops only its receiver and may leave a copied raw key in ongoing registration work.
- Exploit / failure scenario: Leave a removal/backup dialog open past the deadline, or leave a generated password in an unapplied editor then lock. In-progress passkey work can retain an ExistingKey independently of Session. The native modal scenarios were not executed.
- Fix: Move blocking work off the UI thread, make expiry cover dialogs and workers, cancel/expire key-bearing jobs, and seal or explicitly confirm editor drafts during lock. Define how child-viewport activity affects the deadline.
- Confidence: likely (source review).

### [Low] L1 — An icon sizes attribute can panic a debug worker
- Where: `app/src/siteicons.rs:131` (`icon_links`); `app/src/jobs.rs:36`.
- What: Apple-touch-icon score uses unchecked 500 + size.max(180). u32::MAX overflows in the debug build. Release arithmetic wraps instead, so the observed panic is not asserted for release. A disconnected Job is indistinguishable from a still-running job.
- Exploit / failure scenario: sizes='4294967295x1' causes a caught debug panic in the bounded test. Worker progress/error reporting can then be misleading.
- Fix: Clamp/saturate scores, and report disconnected/panicked jobs as terminal failures.
- Confidence: confirmed (reproduced/tested).

### [Info] I1 — Two text/font dependencies are unmaintained
- Where: `Cargo.lock:3452,4088`; `app/Cargo.toml`.
- What: rustybuzz 0.20.1 matches [RUSTSEC-2026-0206](https://rustsec.org/advisories/RUSTSEC-2026-0206.html); ttf-parser 0.25.1 matches [RUSTSEC-2026-0192](https://rustsec.org/advisories/RUSTSEC-2026-0192.html). Both are maintenance notices, not demonstrated vulnerabilities. No other active advisory matched the lockfile in the recorded snapshot, including the requested ureq/image/resvg/keepass/zeroize review targets.
- Exploit / failure scenario: Future text/font bugs may require migration or downstream maintenance. There is no demonstrated exploit from these notices.
- Fix: Plan dependency migration, add repeatable advisory and source/license policy checks, and refresh them immediately before release.
- Confidence: confirmed (reproduced/tested by dependency matching).

### [Info] I2 — Packaging and license metadata are incomplete
- Where: `Cargo.toml:5`; `app/Cargo.toml:1`; `crates/vault-core/Cargo.toml:1`; `crates/passkey-win/Cargo.toml:1`; `scripts/deploy-test.ps1`.
- What: No MSI project, signing pipeline, license/third-party-notice package or release verification workflow was found. workspace.package.license is not inherited by the three packages, so Cargo metadata reports no license for them. README still centers the legacy plugin and incorrectly states that PRF output never leaves the phone, although the desktop receives it.
- Exploit / failure scenario: Signing a binary does not resolve the security findings or establish installation/upgrade behavior. Release users could receive inaccurate security claims or incomplete notices.
- Fix: Finish explicit package metadata, notice inventory and MSI/signing/upgrade validation; update the threat-model documentation. Test hooks are cfg(test), and no release-accessible test credential bypass was found.
- Confidence: confirmed (source and metadata inspection; installer not available to test).

## Remediation on `audit/fixes`

The report was committed first (`1bce765`), followed by one commit for each confirmed high finding. All six demonstrated high-severity failure paths are addressed. This statement is narrower than a claim that the application erases every process/OS allocation or is ready to ship.

| Finding | Commit | Implemented change and regression coverage |
|---|---|---|
| H1 | `81ed196` | A masked TextBuffer prevents real master/entry values entering egui undo snapshots, including when revealed. Screen transitions clear widget state; EntryForm strings and temporary custom-value maps zeroize. Tests drive actual unlock → lock → Ctrl+Z, check undo state after dropping values, exercise Unicode edits, and prevent masked Copy/Cut from leaking or destroying the actual value. |
| H2 | `4097ea9` | Save failure no longer resets the idle deadline. The app seals dirty database contents as standard encrypted KDBX bytes in memory and drops the open session. Recovery requires the complete credentials, retains dirty/conflict state, pins the original path and blocks ordinary close until reauthentication. Tests cover external replacement, disappearance, missing factor, wrong password, failed retries, and recovery through the actual UI. |
| H3 | `f7e87b9` | A credential-change save re-encrypts the previous on-disk contents under the new credentials before replacing the main file. The regression forces a backup-commit error, verifies the original remains accessible, retries, then verifies current and previous contents both reject the old key. A post-commit metadata failure no longer triggers an incorrect credential rollback. |
| H4 | `9a6101f` | Removal generates a new database key, evaluates and validates each retained authenticator, rewraps its envelope, requires a new exclusive, flushed and verified backup key file, then commits the database and sidecar. Tests use injected PRF results to verify retained-key access, rejection of removed/old envelopes by both current KDBX and .bak, new .keyx compatibility, cancellation without ring mutation, and refusal to overwrite an existing recovery key. |
| H5 | `9959b6f` | Icon requests require HTTPS/443 and same-origin candidates. Automatic redirects and proxies are disabled. The connector consumes the exact DNS addresses validated by a resolver that rejects empty, mixed-public/private and non-public answers. Tests reject private/cross-origin candidates and injected private/mixed DNS answers, and verify checked public addresses are passed through unchanged. |
| H6 | `794871c` | Downloaded SVGs have both filesystem/string and embedded-image href resolvers disabled. Tests confirm local-image and embedded-image inputs cannot load pixels, while ordinary vector SVGs still convert. Trusted bundled icons use their separate rendering path. |

### Verification of the fixed branch

Validation below is for product source through `4097ea9`, rebuilt after the baseline evidence run so no baseline local-package artifacts were reused:

- `cargo build --offline --locked`: passed. [Build output](audit-evidence/fixed-build-results.txt).
- `cargo clippy --offline --locked --workspace --all-targets -- -D warnings`: passed, zero warnings. [Clippy output](audit-evidence/fixed-clippy-results.txt).
- `cargo test --offline --locked --workspace -- --test-threads=1`, with `AXV_INTEROP_DIR` pointing to the synthetic KeePass fixtures: **101 passed, 0 failed, 2 intentionally ignored** (48 app, 4 passkey-win, 47 core, 2 genuine interop tests). The ignored tests are live website fetching and preview rendering. [Test output](audit-evidence/fixed-test-results.txt).
- KeePassLib verification of Rust output: **37 assertions passed**. Legacy plugin build/tests: **9 passed**. [KeePass output](audit-evidence/fixed-interop-results.txt), [plugin output](audit-evidence/fixed-plugin-results.txt).
- The repaired, pinned baseline evidence runner also passed all **14** vulnerability-presence assertions. The fixed branch's added security regressions are part of the normal workspace suite, not that baseline harness.
- No dependency or lockfile changes were needed; the baseline advisory/license inventory still applies. No release binary, signed package or MSI was tested.

### Behavior changes and remaining limits

- **Formats:** KDBX 4, AxentraVault/1 and /2, and KeePass XML keyfile v2 are unchanged. The auto-lock recovery buffer is standard encrypted KDBX held only in this process; it is not a new on-disk format. The existing KeePassLib round-trip remains the compatibility check; KeePassXC itself was not run.
- **Memory / editing:** Secret-capable editor fields deliberately disable undo and keyboard copy/cut; the UI explains that the vault's explicit Copy controls provide clipboard access. Revealed text still passes through font/rendering/accessibility machinery. The H1 fix closes the demonstrated persistent undo route and wipes identified application form copies; it is not proof of complete heap, input-event, GPU, OS or worker-memory erasure.
- **Failed saves:** Memory-only recovery is lost if the process terminates or power fails. The locked UI warns to unlock and save before closing and cancels ordinary close requests while recovery exists. If even KDBX serialization/verification fails, the app still locks and explicitly reports that unsaved changes could not be retained. Unapplied editor drafts and blocked native dialogs remain M7; this is not a crash-recovery journal.
- **Key changes:** Retained authenticators must be available during revocation, and older backup key files stop opening the current vault. Old database copies and cloud history retain their original credentials; the changes cannot revoke those copies. A sidecar commit failure after KDBX rekey leaves the verified new backup key file as the recovery route. Physical authenticator prompts, crash/power-loss ordering and filesystem races were not proven by the injected-evaluator tests.
- **Icons:** Sites requiring redirects, a different icon CDN, non-default ports or a configured proxy can now fail to provide an icon. SVG raster image inclusions are disabled. General SVG resource budgets and gzip expansion (M1) remain unresolved.
- **Other findings:** M2 is partially mitigated by exclusive recovery-key creation, flush/read-back verification and sidecar-temp flushing; a full restart/power-loss protocol and path-identity analysis remain open. M3 now treats missing/failed metadata as a conflict, but its concurrent-writer race remains. H1 also blocks the demonstrated editor CopyText route in M6, but native clipboard ownership, marker failures, retry behavior and clearing on lock remain open. M4, M5, M7, L1, I1 and I2 remain open.

## Looks good

- Envelope construction has a conventional encrypt-then-MAC design: fresh random IV, HKDF-SHA256 separation into distinct AES/HMAC keys, length-prefixed fields, and authentication before CBC unpadding. Rust verify_slice uses the MAC library's constant-time tag check. Wrong-secret and every-field tamper tests pass; C# cross-opening passes. This removes the unauthenticated CBC padding-oracle route reviewed here; “impossible in every environment” would be a stronger claim than the evidence.
- Reusing a public PRF input across independently keyed credentials does not equate their PRF outputs. Credential-specific authentication and random PRF secrets remain required. Label lines and ring membership are not authenticated: accept labels only as editable UI hints, not proof of key ownership. Credential deletion/replay has the H4 consequence; altering authenticated fields or substituting an unrelated attacker's envelope does not by itself open the KDBX.
- On the inspected x64 ABI, HmacSecretSaltValues.cred_salts correctly points to a contiguous array of CredWithSalt structs, not an array of pointers. Input vectors/UTF-16 buffers stay alive without reallocating across the blocking calls. Requested version-6 option prefixes and UV_REQUIRED=1 agree with the SDK. Success-path attestation/assertion objects are freed, including assertion semantic-error branches. HRESULT errors return before dereferencing outputs. RAII guards and explicit null/length checks would improve defensive maintenance; a compromised OS was out of scope. HWND is passed as a handle, not dereferenced by Rust.
- Normal save serializes and reopens before replacement, flushes the KDBX temp file, and preserves an older file. Rename-over-existing actually works on this NTFS host, as exercised by save and interoperability tests. A failure during .bak copy ordinarily leaves the original untouched. Power-loss durability of the backup/name replacement was not established and should not be conflated with atomic visibility.
- Ordinary setup error ordering provides recovery: backup-write failure precedes re-key; a normal database-save error restores old in-memory credentials; a sidecar-write error tells the user to use the backup. This positive result does not cover M2 power-loss, alias, or post-rename error ambiguity.
- The display-only header reader caps file input at 64 KiB and bounds slices. On the inspected 64-bit target, u32 field lengths do not create the suspected usize-overflow path; no header indexing bug was found. Checked addition would improve portability. Key/file parser budget concerns remain in M4.
- Explicit raster image decoding sets 4096x4096 dimensions and a 64 MiB allocation limit. Non-image payloads are rejected. Those settings do not prove budgets for the separate SVG path or aggregate workers. Entry path/query/userinfo is discarded before original-host fetching; punycode ASCII names are accepted and raw Unicode hosts are rejected rather than IDNA-normalized. Parent-domain fallback naively drops to the last two labels, so public-suffix correctness also deserves improvement.
- Session owns the activity/toast/editor data and the locked render path does not invoke Session rendering. Existing locked-screen tests pass. This is a visual-isolation result, not proof of memory erasure (H1). Immediate editor viewports cease being submitted after Session is dropped; native window teardown was not inspected.
- Settings writes were limited to paths, layout, column names/sort and timing data, consistent with the brief. No password/entry values are intentionally serialized there. Column names and paths remain visible metadata.
- Launch URLs are restricted to HTTP(S) prefixes before ShellExecuteW; no shell-command interpolation or executable/path launch route was found in that function. OS/browser behavior beyond dispatch was not tested.

## Not reviewed

- Physical phone/YubiKey ceremonies, provider cancellation behavior, older Windows API versions, x86/ARM layouts, hardware authenticators, native clipboard/history/cloud behavior, secure desktop, or window-station isolation.
- Power cuts, low-disk fault injection, NTFS/OneDrive/SMB recovery matrix, concurrent-writer stress, installer upgrade/rollback/uninstall, code signing and Smart App Control.
- A full dependency source-code audit, cipher implementation audit, long-running fuzz campaign, or proof of process/GPU/OS memory erasure. Protected KeePass values and DatabaseKey zeroize on drop; ordinary strings, decoded XML/crypto intermediates and framework allocations still require careful accounting.
- Full legacy-plugin lifecycle/security audit beyond envelope compatibility, keyfile handling, FFI comparison and its tests.
- Recommended fuzz targets: bounded KeyRing/Envelope parsing with duplicate fields and malformed base64; all header truncation points and maximal lengths; raw_key vs keepass differential parsing; HTML attributes/URL resolution; image/SVG decoding in a process with memory/time limits. Fuzzing should never run on real vault files.

## Release-readiness verdict

**Don't ship yet.** The demonstrated H1–H6 paths have finding-specific fixes and regressions on `audit/fixes`. Resolve or explicitly disposition the remaining availability, clipboard, recovery and parsing findings, validate physical authenticators and native dialogs, and finish packaging/signing checks before a release. Build, headless tests and interoperability alone do not establish the broader memory-erasure or power-loss guarantees.

