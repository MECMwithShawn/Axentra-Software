# Axentra Vault second-pass security audit brief

You audited Axentra Vault at `a264271` and fixed H1–H6 on `audit/fixes`. Your verdict was **don't ship**.
Since then the in-session engineer (Claude) has corrected two of your fixes, added new security features
and added release packaging. This brief asks you for a second pass before the first public release,
`vault-v0.1.0`.

The owner and stakes haven't changed. One person uses the app daily for a real database of about 300
entries (banking, government, work). Assume real money and identity are at stake.

- First brief: `docs/AUDIT_BRIEF.md`. Your report: `docs/AUDIT_REPORT.md`. Evidence: `docs/audit-evidence/`.
- **Deliverable:** `docs/AUDIT_REPORT_2.md` plus fix commits on `audit/fixes` (see §9).

---

## 1. Purpose and scope

**Verdict sought for v0.1.0:** **ship**, **ship after fixes** (name them), or **don't ship**.

| Item | Value |
|---|---|
| Branch | `audit/fixes`. Stay on it; don't merge into `main`. |
| HEAD | `043c6e3` |
| New code | `194d9b2..043c6e3`: 9 commits (§2) |
| Re-verify | Your fixes as they stand at HEAD: `81ed196` H1, `4097ea9` H2, `f7e87b9` H3, `9a6101f` H4, `9959b6f` H5, `794871c` H6 |
| Fix | M1–M7, L1, I1 and I2 from your report, plus any new findings you confirm (Part C) |
| Release | Installers, packaging and signing status (Part D) |

**Threat model:** the same as the first brief (§3), with three changes:
1. **Same-user, non-admin malware while the vault is unlocked** is now partly in scope. `9418be0`, the
   README and the Settings screen all claim defences against it: a process access list, mitigation
   policies and capture exclusion. Judge those claims as written. Admin or kernel malware stays out of scope.
2. **A second outbound network feature:** the opt-in Have I Been Pwned check.
3. **The install itself:** per-user installers and unsigned binaries.

Work in order: Part A, then B, C and D. If you confirm a Critical, report it before continuing.

## 2. What changed since your report

| Commit | Change | Why |
|---|---|---|
| `e1fbc66` | Icons: redirects are followed by hand (at most 5 hops), only to HTTPS:443 URLs on the same site. `site_of` takes the last two labels, or three for 2-letter-ccTLD second-level domains such as `co.uk`. Every hop still goes through `PublicResolver`. Icon `<link>`s may point anywhere on the same site. `read_capped` caps the **decoded** HTTP body (M1; SVGZ decoded inside usvg isn't capped, see H6.1). The apple-touch score saturates (L1). | **Corrects your H5 fix.** `max_redirects(0)` broke icons for 9 of 13 sampled real sites (e.g. `chase.com` → `www.chase.com`). After the change, 13 of 13 fetched an icon. Both figures come from manual runs; the 13 hosts aren't recorded. `siteicons::live_fetch` covers only 5 hosts, none of them a bare-domain → `www` redirect (consider adding `chase.com`). |
| `2daf909` | The editor's Title, Username and URL (and custom-field names) go back to a plain `TextEdit`. Password, Notes and every custom-field value keep `secret_edit`, protected or not (unprotected ones are shown revealed), as does the Unlock screen's master password. | **Corrects your H1 fix.** The masked buffer on non-secret fields removed Ctrl+C and undo there. Lock already clears all egui data (`data.clear()` in `app.rs`). |
| `158fe7f` | New ignored test `ui_tests::store_screenshots` renders neutral demo screenshots. | Images for the product page. Test-only. |
| `9418be0` | New `app/src/hardening.rs`: <br>• process DACL `D:P(A;;GA;;;SY)(A;;0x101001;;;OW)` set with `SetKernelObjectSecurity`<br>• `SetProcessMitigationPolicy`: extension points disabled; image load set to NoRemote \| NoLowMandatory \| PreferSystem32<br>• `WDA_EXCLUDEFROMCAPTURE` on every window in the process, re-applied about once a second (setting in `capture.txt`, on by default)<br>• antivirus health and Smart App Control state shown in Settings, with a warning on unlock if antivirus needs attention | Defends against same-user malware. The signature policy isn't used because the OpenGL driver is vendor-signed. Checked on the real app: `OpenProcess(PROCESS_VM_READ)` from another process returns `ERROR_ACCESS_DENIED`, `QUERY_LIMITED` is still granted, and a screen capture left out the window. Side effect: PowerShell `Stop-Process` can't end the app (Task Manager and `Process.Kill` can). |
| `cc3ca44` | New `app/src/breach.rs`: opt-in HIBP range API (5-hex SHA-1 prefix, `Add-Padding: true`) behind a confirmation dialog. Results stay in session memory, keyed by SHA-1. Adds `Issue::Breached` to `vault-core` `organize.rs`, plus Watchtower and Classic Security-menu UI. | New feature. The live ignored test passed. |
| `5184a15` | README rewritten. | Also fixes the README's claim, flagged in I2, that the PRF output never leaves the phone. |
| `c8aee0b` | Release profile: `panic = "abort"` removed (so it unwinds) and `strip = true`. Each crate inherits license and authors. Adds a GPL-3.0 `LICENSE`, `packaging/windows/AxentraVault.iss` (Inno, per-user), `AxentraVault.wxs` (WiX v7, `Scope="perUser"`) and `scripts/package.ps1`. | Unwinding lets `Drop` zeroize on a panic and keeps a job panic inside that job. Both installers were installed silently and the app was run from each. The MSI uninstalled cleanly. **The Setup.exe install is the owner's working copy.** |
| `ae0a6e0` | Tray icon (`app/src/tray.rs`, tray-icon 0.19.3) with Show / Lock vault / Exit; hidden while Axentra Command holds `Local\Axentra.TrayHost` (checked every 2 s). Single instance per settings folder (`app/src/instance.rs`): a second launch forwards `--show` (default), `--lock` or `--exit` to the running copy over named events and exits. `axentra.app.json` registers Vault with Command. | The owner wants every Axentra app in Command's tray, with its own icon when Command isn't running (protocol: `~/code/axentra-command/docs/TRAY-HOST.md`). Named events because the process DACL blocks other channels. |
| `043c6e3` | Test `app::security_tests::external_lock_request_locks_and_saves`. | Covers the Lock request path. |

Notes:
- `e1fbc66` also touches `app.rs`, `jobs.rs`, `session.rs`, `vault.rs` and `main.rs`. Those hunks are only
  rustfmt changes and reordering: with whitespace removed, the files are identical apart from braces and
  trailing commas. Review only its `siteicons.rs` changes and the `flate2` dev-dependency it adds to
  `app/Cargo.toml` (and `Cargo.lock`) for the gzip-cap test.
- `floating.rs` in this brief means `app/src/session/floating.rs` (module `session::floating`).
- **Line numbers** (`~L…`) were taken at `c8aee0b`, before the tray commit. At HEAD, `app.rs` lines after
  ~L60 are about 40 lower (e.g. `data.clear()` is at ~L297), and `session.rs` lines are about 25 lower
  between ~L430 and ~L2100, and about 7 lower after that. Function names are authoritative; search for them.
- The Classic column chooser (`columns.rs`, `columns.txt`), sortable columns (`sort.txt`) and maximized
  windows were already in `a264271`. Your first pass only checked that settings hold no values. Part B asks
  more specific questions about them.

## 3. Build and test

Toolchain: Rust 1.99.0 MSVC on Windows 11 Home (Windows Sandbox isn't available). Run everything from
`C:\Users\shawn\code\axentra-vault`.

```powershell
git rev-parse --short HEAD; git status --short             # 043c6e3; only "?? docs/AUDIT_SECOND_PASS.md"
$sb = "$env:TEMP\axv-audit2"; New-Item -ItemType Directory -Force "$sb\appdata","$sb\vault","$sb\dist-before" | Out-Null
$env:APPDATA = "$sb\appdata"        # REQUIRED before any test or app run (see below)
cargo build
cargo test --workspace              # expected: 110 passed (57 app, 4 passkey-win, 47 core, 2 interop)
cargo clippy --workspace --all-targets -- -D warnings      # expected: clean
```

This brief is untracked at `043c6e3`. Leave it out of your fix commits.

**Why `APPDATA` matters:** the `app.rs` `security_tests` (~L669) run `VaultApp` through the real Unlock
screen, and unlocking calls `settings::remember_database` (~L211) and `settings::set_unlock_ms` (~L366).
`Session::new` also reads `columns.txt` and `sort.txt` (`session.rs` ~L229). Only `ui_tests` redirects
`APPDATA` (`isolate_settings`, once per process). Depending on test order, these tests can write the owner's
real `%APPDATA%\Axentra Vault\last-database.txt` and `unlock-times.txt` (vault paths), and depend on the
owner's column and sort settings. Treat this as a finding and fix it.

If your shell tool starts a new process per command, `$env:APPDATA` is lost between commands. Set it in the
same command as every cargo or app invocation (e.g. `$env:APPDATA="$env:TEMP\axv-audit2\appdata"; cargo test --workspace`).
The app pre-fills the unlock screen from `last-database.txt` (`app.rs` ~L64): if it ever shows a path
outside `$sb`, close the app without unlocking.

The 2 interop tests do nothing unless `AXV_INTEROP_DIR` is set; `interop\run.ps1` sets it.

**Ignored tests:** run them one at a time. Don't run `--include-ignored` across the workspace: that runs
the hardening child test inside the shared test process, plus every network test.

| Command | Effect |
|---|---|
| `cargo test -p axentra-vault breach::tests::live_check -- --ignored --exact` | One GET to `api.pwnedpasswords.com/range/5BAA6` |
| `cargo test -p axentra-vault siteicons::live_fetch -- --ignored --exact --nocapture` | Fetches icons from public sites; writes `app/assets/preview/` (gitignored) |
| `cargo test -p axentra-vault ui_tests::render_previews -- --ignored --exact` | Preview PNGs (gitignored) |
| `cargo test -p axentra-vault ui_tests::store_screenshots -- --ignored --exact` | Neutral demo screenshots (gitignored) |
| `hardening::tests::child_restricted_process_refuses_memory_access` | Don't run it directly. `hardening_blocks_memory_reads` runs it in a child process. |

**KeePass interop.** `interop\run.ps1` defaults to `C:\Program Files\KeePass Password Safe 2\KeePass.exe`.
That file no longer exists: KeePass was uninstalled and only `Plugins\` remains. Use the cached
KeePass 2.61.1 at `interop\out\KeePass.exe`, but copy it somewhere else first. `run.ps1` copies its
`-KeePassExe` into `interop\out`, and copying a file onto itself fails.

```powershell
Copy-Item interop\out\KeePass.exe "$sb\KeePass.exe"; (Get-AuthenticodeSignature "$sb\KeePass.exe").Status  # Valid
.\interop\run.ps1 -KeePassExe "$sb\KeePass.exe"            # regenerates interop\out\fixtures (gitignored)
.\keepass-plugin\build.ps1 -KeePassExe "$sb\KeePass.exe"   # legacy plugin: expected 9 PASS
```

The legacy plugin parses the same `.axvault` envelope (`keepass-plugin\src\Envelope.cs`). Rerun its build
after any `Envelope`/`KeyRing` change (M4) so the C# and Rust parsers don't drift. Its `-KeePassExe` default is
the same missing path.

**Running the app:** make a test vault with
`cargo run -p vault-core --example make_demo -- "$sb\vault\demo.kdbx"` (password `demo`). Start
`target\debug\axentra-vault.exe` from the shell where `$env:APPDATA` is set. The debug build has a console:
it must print neither `process access hardening failed` nor `mitigation policy not applied`. The owner's
copy may be running. Filter every process lookup by `Path`, and never kill or attach to the owner's copy.

- **Own instances only.** Run access-matrix and termination tests (B1.2, B1.10) only against a process you
  started. Get its PID with
  `Get-Process axentra-vault | ? Path -eq "$PWD\target\debug\axentra-vault.exe"` and target that PID
  (`taskkill /PID <pid> [/F]`, `[Diagnostics.Process]::GetProcessById(<pid>).Kill()`). Never use
  `taskkill /IM`. `Stop-Process` fails on the hardened app, so stop each of your instances by PID before
  rebuilding (a leftover instance locks `target\debug\axentra-vault.exe`).
- **Desktop-wide tests.** UI Automation tools walk every window, full-screen captures (Win+Shift+S, Graphics
  Capture) include every window and land in the shared clipboard and its history, and clipboard tests
  overwrite what the owner copied. Before any desktop, UIA, screen-capture or clipboard test, ask the owner
  to close their copy, then confirm that no `axentra-vault` process runs from
  `%LOCALAPPDATA%\Programs\Axentra Vault*`. Scope UIA and `PrintWindow` to your own instance's HWND or PID.
- **No elevation.** Don't use an admin shell or `SeDebugPrivilege`: an elevated tool can reach the owner's
  running copy. The hardened DACL denies `VM_READ` to your own instance too, so judge memory questions
  (H1 Q3, B2.3) from source and in-process tests, and mark dump-based checks Not tested. A debug-only switch
  that skips hardening would itself need review; propose it as a finding rather than adding it silently.

**Packaging:** `.\scripts\package.ps1` builds the release exe at `target\release\axentra-vault.exe` (it isn't
copied to `dist\`), then writes `AxentraVault-0.1.0-Setup.exe`, `AxentraVault-0.1.0.msi`, a `git archive`
source zip (`AxentraVault-0.1.0-source.zip`) and `SHA256SUMS.txt` into `dist\`. It **deletes
`dist\AxentraVault-0.1.0*` first**, and the files there now are the build the owner installed, so back them up
with `Copy-Item dist\* "$sb\dist-before"` (plus `target\release\axentra-vault.exe` if you need the exact
packaged binary). It passes `-acceptEula wix7`, which accepts the WiX v7 EULA: ask the owner before running
it, or report packaging as Not tested. **Never run the Setup.exe, `msiexec /i`, or
`scripts\deploy-test.ps1` on this PC.** Both installers target `%LOCALAPPDATA%\Programs\Axentra Vault`,
which is the owner's live install. `deploy-test.ps1` overwrites `...\Axentra Vault (Test)` and a desktop
shortcut. **No VM is available here** (Windows 11 Home: no Hyper-V or Sandbox; no VirtualBox or VMware), and
you may not create Windows accounts. Report every VM, install, upgrade and uninstall item as Not tested,
with the exact test plan, unless the owner provides a VM.

---

## 4. Part A: re-verify H1–H6 and the follow-ups

For each finding, confirm the fix still holds at HEAD, answer the questions, and file anything new.

### H1: secrets kept in egui undo history (`81ed196`, partly reverted by `2daf909`)
- **Code:**
  - `secret_edit.rs`: `MaskedBuffer` (~L7–75; `replace_with` is a no-op, `type_id`) and `show()` (~L77–118: Copy/Cut filter, reveal layouter, `clear_undoer`).
  - `app.rs`: `ctx.data_mut(|data| data.clear())` (~L255).
  - `session.rs::editor_ui` (~L1646): the `full` closure for Title/Username/URL (~L1663), plus the password, notes and custom fields.
  - `form.rs`: `EntryForm` and its `Drop`.
- **Tests:** `app::security_tests::master_password_cannot_be_undone_after_lock` and these `secret_edit::tests`:
  - `unicode_edits_never_expose_plaintext_through_text_buffer`
  - `hidden_and_revealed_fields_leave_no_undo_secret_after_drop`
  - `masked_edit_neither_copies_masks_nor_loses_values_on_cut`

1. Since `2daf909`, `data.clear()` is the only thing that removes Title/Username/URL undo history at lock.
   No test covers it alone; the master-password test would pass without it. Add one. Does `data.clear()`
   leave anything that matters, such as the font galley cache, focus memory or input buffers?
2. `2daf909` ignores KeePass's `title_protected`, `username_protected` and `url_protected` flags
   (`form.rs` ~L27–29). Should a protected username use `secret_edit`? Also: edit entry A, then open
   entry B without locking. The same viewport keeps stable auto ids. Does Ctrl+Z in B's Title bring back
   A's value?
3. **Reveal mode:** the layouter makes an unzeroized `to_string()` on every layout, and the galley and the
   AccessKit text runs carry the plaintext. This applies to every unprotected Notes field and custom-field
   value too, since they go through `secret_edit` in reveal mode (`session.rs` ~L1746, ~L1782), not only to
   protected fields the user has revealed. Revealed detail-panel labels behave the same way. UI
   Automation isn't blocked by the process DACL. Assess this against the new same-user claims.
4. `MaskedBuffer::type_id` returns `TypeId::of::<String>()`. egui 0.36.2 doesn't seem to downcast
   internally, but any downcast would be undefined behaviour. Replace it with a private marker type?
5. **IME:** winit 0.30.13 `set_ime_purpose` does nothing on Windows (`platform_impl/windows/window.rs`
   ~L932), so masked fields don't turn off IME/TSF. How do preedit and commit events map onto
   `MaskedBuffer`? Is preedit text kept anywhere?

### H2: idle lock failed open when saving failed (`4097ea9`)
- **Code:**
  - `session.rs::frame_logic` (~L437: activity at ~L440, idle branch at ~L448–467) and `key_job_pending` (~L2221).
  - `vault.rs`: `SealedVault` (gained `Clone` in `4097ea9`), `seal`, `open_sealed`.
  - `jobs.rs::unlock`: the recovery branch. `Unlock::start` clones the recovery blob into each unlock worker
    (`app.rs` ~L661), and a worker can outlive its attempt.
  - `app.rs`: Session → Unlock handoff (~L231–237); close guard in `Unlock::show` (~L345).
- **Tests:**
  - `session::idle_lock_tests::failed_save_still_locks_and_recovers_only_with_complete_credentials`
  - `app::security_tests::idle_lock_recovery_uses_the_real_unlock_screen_even_if_file_disappears`

1. Idle lock is skipped while `passkey_job` or `rotation_job` is set. `Job::poll` maps `Disconnected` to
   `None` (`jobs.rs` ~L36). Now that the release build unwinds (`c8aee0b`), a panicking key worker leaves
   its job set forever, which turns idle lock off for the rest of the session. Before `c8aee0b` the process
   aborted instead. Confirm this. The fix is spec 1 in Part C.
2. `frame_logic` reads `ctx.input` from the root viewport only. Does typing in the `entry-editor` immediate
   viewport (`session.rs` ~L1628) refresh `last_activity`? If it doesn't, 5 minutes of typing in the
   editor locks the vault and drops the draft (M7). Check this on the desktop, because kittest may embed
   viewports.
3. After a failed save, `seal()` serializes and verifies (two KDF runs) on the UI thread. How long does that
   take with heavy Argon2 settings, and what happens if memory runs out? `seal()` refuses while
   `previous_credentials` is pending. Confirm that state can't be reached: the only production
   `set_credentials` caller is `finish_key_change` (~L2380), and it rolls back on failure.
4. **Recovery is memory-only.**
   - The locked screen says "explicitly discard" but has no discard control.
   - Passkey recovery needs `<db>.axvault` at the original path, which may be missing in exactly this case.
   - After recovery, "Save anyway" overwrites the other writer, whose version survives only in `.bak`.
   - "Save and lock" that hits `ChangedOnDisk` → Overwrite saves but doesn't lock (`confirm_dialog`, ~L1987).
   - Copies of the `SealedVault` blob made per unlock attempt: confirm none outlives a successful unlock in
     a way that matters.

   Are these acceptable for v0.1.0 if documented?

### H3: first passkey setup left a password-only backup (`f7e87b9`)
- **Code:**
  - `vault.rs`: `set_credentials` (~L156) and `save_overwriting` (~L182–227: re-key ~L188, `.tmp` ~L202, `.bak` commit ~L208–218, main rename ~L219).
  - `verify_db` compares counts only.
  - `set_credentials` decides whether a re-key is pending with the `PartialEq, Eq` that `f7e87b9` derived on
    `Credentials`: a byte-wise, non-constant-time compare of passwords and raw key-file bytes, not of the
    effective KeePass key.
- **Tests:**
  - `vault-core` `vault::tests::credential_change_protects_previous_contents_and_can_fail_safely`
  - `vault-core` `vault::tests::rekey_with_passkey_bytes_matches_backup_keyfile`

1. The re-keyed `.bak` is renamed into place **before** the main rename. Suppose the main rename then fails
   (AV, OneDrive or an indexer holds the file). The main file is still on the old credentials, `.bak` is on
   the new ones, the previous `.bak` generation is gone, and the rollback message says "old credentials".
   Can the user recover from that state, and is the message accurate?
2. The re-key re-reads the file on disk and opens it with the old credentials. If the file changed since it
   was opened (the M3 race), the app re-keys someone else's content. If another client already re-keyed
   it, the change fails with an error that reads like a wrong password.
3. Does any artifact stay on the old key? For example `.axvault.old` combined with an old key file, or a
   leftover `<db>.tmp`.
4. Up to five KDF runs happen on the UI thread. Is a count-only `verify_db` enough?
5. Two encodings of one key (32 raw bytes vs XML `.keyx`) count as a credential change. Does `set_credentials`
   stay correct after the M5 `raw_key` change (compare effective keys?), and does the non-constant-time
   compare matter?

### H4: removing a passkey didn't revoke access (`9a6101f`)
- **Code:**
  - `session.rs`: `remove_unlock_key` (~L2227), `poll_passkey_job` (~L2276), `finish_key_change` (~L2344: backup dialog loop, `write_new_backup`, `set_credentials` + `save`, `write_ring`), `write_ring` (~L2407, still copies the old ring to `.old`) and `write_new_backup` (~L2489).
  - `jobs.rs`: `rotate_keys` and `rotate_keys_with` (~L240–300).
- **Tests:**
  - `jobs::rotation_tests::revocation_rotates_retained_keys_and_rejects_old_envelopes`
  - `jobs::rotation_tests::cancelled_rotation_does_not_change_the_ring`
  - `session::backup_creation_tests::replacement_recovery_key_never_overwrites_existing_files`
  - `session::floating::tests::added_keys_join_the_ring_without_rekeying`

1. **Ring integrity:**
   - Rotation skips only `index == remove`, and `KeyRing::parse` doesn't reject duplicate credential IDs. Does a duplicate of the revoked key survive?
   - Retained envelopes reuse the same `prf_input`. Is that acceptable?
2. `finish_key_change` writes the ring it computed at click time without re-reading the sidecar. A key that
   a synced device added in the meantime is lost.
3. **Test gaps:**
   - `cancelled_rotation_does_not_change_the_ring` borrows the ring immutably, and its worker touches no files. Does it prove anything?
   - The order in `finish_key_change` and its failure branches are untested, because they sit behind rfd dialogs.
4. **Usability:**
   - Every retained key needs its own PRF prompt.
   - With two keys lost, neither can be revoked, because there's no rotation path that uses only the backup key file.
   - KeePass setups that use the old `.keyx` stop working.

   Should this be documented or fixed?
5. **Backup path:** picking `<db>.tmp`, or `<db>.bak` when none exists yet, gets truncated or replaced by the
   save that follows. `create_new` only checks at creation time (M2).

### H5 and `e1fbc66`: icon requests to private or third-party hosts (`9959b6f`)
- **Code** (`siteicons.rs`):
  - Host and URL checks: `icon_host` (~L23), `public_address` (~L45), `public_uri` (~L74), `site_of` (~L90), `same_site` (~L103).
  - `PublicResolver` (~L116); `agent` (~L138: `max_redirects(0)`, `proxy(None)`).
  - Fetching: `icon_links` (~L195), `get_bytes` (~L375, manual redirect loop), `read_capped` (~L403), `fetch` (~L419).
- **Tests** (`siteicons::tests`):
  - `hosts_are_public_web_names_only`
  - `finds_and_ranks_icon_links`
  - `icon_requests_cannot_escape_the_public_origin`
  - `dns_answers_are_filtered_and_pinned`
  - `decompressed_bodies_are_capped_and_large_icon_sizes_do_not_overflow`
  - ignored: `siteicons::live_fetch`

1. **Parent-domain fallback (source-verified):** `fetch` (~L421–425) falls back to the last two labels, not
   to `site_of`. So `www.bank.co.uk` also requests `https://co.uk/`, and `x.com.au` requests `com.au`, and
   the app may store that domain's icon.
2. **`site_of` boundaries:**
   - Too broad for shared hosting (`*.github.io`, `*.azurewebsites.net`, `*.vercel.app`, `s3.amazonaws.com`).
   - Too narrow for short names under 2-letter TLDs (`fly.io` vs `www.fly.io`, `x.ai`).

   Is the impact limited to icon spoofing or failure? Is that acceptable without the Public Suffix List?
3. **Redirect loop tests:** the loop has no unit test. Add loopback tests for an off-site `Location`, a
   relative `Location`, more than 5 hops, and a hop to a host with a private DNS answer.
4. **Two URL parsers:** `icon_host` uses `vault_core::search::host_of`, while `same_site` and the resolver
   use `http::Uri::host`. Try backslashes, percent-encoding, bracketed IPv6 and trailing dots.
5. **Connector:** confirm the ureq 3.4.2 connector connects only to the `ResolvedSocketAddrs` it was given,
   with no second lookup, and that TLS still checks the hostname.

### H6: downloaded SVGs could open local files (`794871c`)
- **Code:** `siteicons.rs::to_png` (~L270; SVG branch ~L272–295).
- **Tests:**
  - `siteicons::tests::downloaded_svg_cannot_include_local_or_embedded_images`
  - `siteicons::tests::converts_images_to_64px_png`

1. **SVGZ bomb (strongest lead):**
   - The SVG check is only `head.contains("<svg")` over the first 512 bytes (lossy UTF-8), so a gzip stream with `<svg` in its FNAME or FCOMMENT header passes.
   - usvg 0.45.1 `Tree::from_data` sees the gzip magic, and `decompress_svgz` reads the whole stream with no limit (`read_to_end`; source-verified). That means about 1000:1 expansion, across 6 workers.
   - If allocation fails, the process aborts: unsaved changes are lost and H2 recovery is bypassed.
   - It also defeats `read_capped`.

   Reproduce with a bounded size.
2. **No budget for SVG parsing and rendering:**
   - `allow_dtd: true`; nodes and filters are unbounded; the 8 s timeout covers only the network.
   - A hung worker keeps `icon_job` set, which blocks every later icon download (`session.rs` ~L296).
3. **href coverage:** confirm `feImage` goes through the same href resolver. Add test cases for UNC paths,
   `file:///` and relative hrefs.
4. **Custom icons from the database** (`bytes://keepass-icon/{uuid}.png`, `session.rs` ~L264–277) are decoded
   by egui_extras with the image crate's default limits, not with `to_png`'s 4096² / 64 MiB limits.
   This overlaps M4.

---

## 5. Part B: review the new code

### B1. `app/src/hardening.rs` and startup order (`9418be0`)
- **Code:**
  - `restrict_process_access` (~L77) and `apply_mitigations` (~L110; policies 6 and 10, flags passed as a bare `u32`).
  - `set_affinity` / `exclude_from_capture` (~L133–167).
  - `security_status`, `summary`, `warning` (~L194–260).
  - All FFI is `raw-dylib` (kernel32, advapi32, user32, wscapi).
  - `main.rs`: DACL (~L25), mitigations (~L28), the capture flag from `capture.txt` (~L31), then `run_native`.
  - `app.rs::logic` re-applies capture (~L139–149).
- **Tests:**
  - `hardening_blocks_memory_reads` runs the child test, which checks only that VM_READ is denied, QUERY_LIMITED is allowed, and the mitigation calls succeed.
  - `security_status_reads_without_error`.

1. **FFI:** check the signatures, the size argument for policies 6 and 10, the `RegGetValueW` flags,
   `LocalFree` on every path, and the callback ABI.
2. **Access matrix:** from a second process, try `VM_WRITE`, `VM_OPERATION`, `CREATE_THREAD`, `DUP_HANDLE`,
   `QUERY_INFORMATION`, `SET_INFORMATION`, `WRITE_DAC`, `READ_CONTROL` and `PROCESS_ALL_ACCESS`. Only VM_READ
   is tested today. Add a child-process test for the rest. Test only your own instance, by PID (§3).
3. **Threads:** `SetKernelObjectSecurity` covers the process object only. Can a same-user process call
   `OpenThread(GET_CONTEXT | SET_CONTEXT | SUSPEND_RESUME)` on the UI thread? If it can, the README's
   "can't … create threads in it" and the Settings line "can't … inject code into it" (`floating.rs` ~L2255)
   overstate the protection. Options: the token default DACL, or per-thread DACLs.
   **Token:** `QUERY_LIMITED` (granted) is enough for `OpenProcessToken`. From a second same-user process, try
   `OpenProcessToken(TOKEN_QUERY | TOKEN_ADJUST_DEFAULT | TOKEN_DUPLICATE)`. If it succeeds, the token's own
   DACL must be tightened before any default-DACL fix means anything, because malware could set the default
   DACL for every thread and object the app creates later.
   Judge every user-facing security claim against your findings, using the claim inventory in Part D.
4. **Timing:** handles opened before `main()` keep their access. Static imports and their `DllMain` run before
   the mitigations, and `PreferSystem32` only affects later loads.
5. **Who still needs access:**
   - Passkey unlock and registration (USB, NFC, phone hybrid) under the hardened DACL. Only VM_READ and capture were checked. Use only a test authenticator the owner provides (§8).
   - WER dumps.
   - Screen readers.
   - Legacy IMM32 IMEs, which disabling extension points blocks. Check CJK input.

   Does policy 6 also block thread-specific `SetWindowsHookEx` and in-context `SetWinEventHook`?
6. **Routes the defences don't cover:**
   - Per-user COM hijacks (`HKCU\Software\Classes\CLSID`) of in-process COM (the rfd `IFileDialog`, TSF, OLE clipboard), and shell extensions loaded by the file dialog.
   - `WH_KEYBOARD_LL` keyloggers.
   - `SendInput`, which can untick the capture setting and resets idle lock.
   - UI Automation: AccessKit is on, so revealed values, titles, usernames and the Notes column are readable.
   - `GetWindowText`: the editor's window title contains the entry title (`session.rs` ~L1621).

   Sort these into README limits and fixes.
7. **Capture exclusion:**
   - The return value is ignored.
   - It's re-applied at most once a second and only while `logic()` runs, so a new editor viewport is capturable for up to about 1 s, and an rfd modal pauses re-application.
   - Test `PrintWindow(PW_RENDERFULLCONTENT)`, Graphics Capture (Win+Shift+S) and Alt-Tab thumbnails.
8. **Silent failures:** a hardening failure is only `eprintln!`ed, and the release build has no console. The
   Settings text claims protection unconditionally. Separately, `capture.txt=show` in the user-writable
   `%APPDATA%` turns capture exclusion off at the next launch.
9. **Security status:**
   - Smart App Control is read from the registry policy state (`VerifiedAndReputablePolicyState`), not from what is actually enforced. Errors show as "not available".
   - `wscapi.dll` is a load-time import, so does the exe start on SKUs without it?
   - The WSC call runs on the UI thread at every unlock (`session.rs` ~L225).
10. **Termination:** the DACL grants `PROCESS_TERMINATE` (0x101001), yet `Stop-Process` fails. Explain why, and
    test `taskkill /PID`, `taskkill /PID /F` and Restart Manager (the installer sets `CloseApplications=yes`)
    against your own instance only (§3). Test Restart Manager with an `RmStartSession` /
    `RmRegisterResources` test over your debug exe path, never through an installer.
11. **Build on other platforms:** `mod hardening` is `#[cfg(windows)]`, but `main.rs`, `session.rs` and
    `floating.rs` use it without a cfg. The app is Windows-only, so this is Info at most.

### B2. `app/src/breach.rs` and its UI (`cc3ca44`)
- **Code:**
  - `breach.rs`:
    - `agent` (~L42): 15 s timeout, `https_only`, status errors. No `.proxy(None)`; default redirects.
    - `check` (~L54): 4 scoped workers; the first error fails the whole check.
  - `session.rs`:
    - `health` (~L2113) re-hashes every active password on every call once at least one breach was found.
    - `breach_count` (called from `floating.rs` ~L1885) hashes the selected password every frame, and
      `breach_status` calls `health()` again.
    - `start_breach_check` (~L2166): hashes on the UI thread, then a blocking `rfd::MessageDialog` with no parent.
    - `poll_breach_job` (~L2205).
  - `floating.rs`: Watchtower (~L1045) and `security_module` (~L1812).
- **Tests:**
  - `breach::tests::hashes_and_parses_ranges`
  - `session::breach_tests::breached_passwords_show_in_health_and_follow_edits`
  - ignored: `breach::tests::live_check`

1. **M1 again (source-verified):** `breach.rs` ~L82 uses `with_config().limit(4 << 20).read_to_string()`. In
   ureq 3.4.2, `BodyReader` puts the `LimitReader` *below* the content decoder, so the decoded body has no cap.
2. **Proxy and redirects:** the default ureq config reads `ALL_PROXY`/`HTTPS_PROXY` from the environment
   (`config.rs` ~L948) and follows up to 10 redirects. The icon agent sets `.proxy(None)`. Make the two
   consistent?
3. **Memory:** unsalted SHA-1 hex of every password is kept in plain `String`s: the `hashes` Vec, `wanted`,
   the `breached` keys, and recomputed values in `health()` every frame. None are zeroized, and the worker
   outlives a lock because jobs can't be cancelled. A weak password's SHA-1 is effectively the password.
4. **Panics:** a panicking worker propagates through `thread::scope` and disconnects the job. `breach_job`
   then stays `Some` forever: the button stays disabled, a spinner repaints indefinitely, and no error is
   shown (L1).
5. **Privacy:** the set of prefixes (about one per distinct password) is identical on every check and tied
   to the user's IP, and the User-Agent names the app. Is the dialog text ("your passwords never leave this
   PC") accurate enough?
6. **Minor:** ureq error text, which may contain the prefix URL, is shown in the UI. `Issue` derives `Ord`
   with `Breached` last; check anything that ranks by it. `security_module` doesn't filter out archived or
   recycle-bin entries, so its header and its breach note can disagree.

### B3. Columns, sort and settings (baseline code, not examined closely before)
- **Code:**
  - `columns.rs::parse` (~L66) accepts any `custom:<name>`.
  - `settings.rs` uses plain `fs::write` and ignores errors.
  - `session.rs`: `custom_field_names` (~L881), `row` (~L897; protected Notes and custom fields show a fixed mask) and `sort_rows` (~L2434; sorts on the displayed text).
- **Tests:**
  - `columns::tests::round_trips_and_orders`
  - `ui_tests::classic_columns_can_be_shown_and_hidden`
  - `ui_tests::classic_columns_sort_and_right_click`

  No test checks that protected cells are masked.

1. **`custom:Password` (source-verified):** if `columns.txt` contains `custom:Password`, `row` shows
   `e.fields["Password"]` unless that value is flagged protected. A KDBX written with password memory
   protection off would then show every password in a sortable column. Reject built-in field names and
   add a masking test.
2. **OTP:** `custom_field_names` excludes `KNOWN_FIELDS` but not `fields::OTP` (`search.rs` ~L130 does
   exclude it), so OTP seeds are offered as columns and shown in plain text if unprotected.
3. **Notes column:** unprotected Notes shows the first line of every note at once, including over UIA.
   Is that acceptable?
4. **Names on disk:** custom field names (e.g. "Recovery codes") are written to `columns.txt` and
   `sort.txt`, but the `settings.rs` header says "Nothing about vault contents". Fix the comment or the
   behaviour.
5. **Per-frame work:** `row` builds plaintext `extra` strings every frame, and `custom_field_names` scans the
   whole vault every frame.

### B4. Windows, capture and the unlock warning (`app.rs`)
1. `logic` repaints every 500 ms while open, and every 1 s for capture. Confirm neither creates input events,
   which would reset idle lock.
2. On lock, confirm the screen switches before any resize, so no vault data is drawn during the transition.
3. The antivirus warning is dropped when a recovery notice is shown (~L213–217). Is that acceptable?

### B6. Tray icon, single instance and the command channel (`ae0a6e0`)
- **Code:**
  - `instance.rs`: `command_from_args`, `base_name` (FNV-1a of the lowercased `%APPDATA%`, giving
    `Local\Axentra.Vault.<hash>`), `start` (`CreateMutexW` + `ERROR_ALREADY_EXISTS`, three auto-reset
    events `.Show/.Lock/.Exit`), `signal` (`OpenEventW(EVENT_MODIFY_STATE)` with retries,
    `AllowSetForegroundWindow(ASFW_ANY)` for Show), `Instance::listen` (`WaitForMultipleObjects` thread).
  - `tray.rs`: `command_is_hosting` (`OpenMutexW(SYNCHRONIZE, "Local\Axentra.TrayHost")`), `Tray::new`
    (global `MenuEvent`/`TrayIconEvent` handlers that send a `Command` and request a repaint), `Tray::update`.
  - `main.rs`: the instance check runs **before** `restrict_process_access`; `--lock`/`--exit` with no
    running copy exits without opening a window.
  - `app.rs`: `VaultApp::new` (tray and listener; tests use `with_layout`, which creates neither), the
    command loop in `logic`, `lock_requested` handled in `ui`. `session.rs::lock_now` is the auto-lock path
    (save, or seal on failure) shared with idle lock.
  - `axentra.app.json`: discovered by Command and overrides its built-in `apps/vault.json`.
- **Tests:** `instance::tests::*` (argument parsing, stable names, a forwarded signal reaches the event) and
  `app::security_tests::external_lock_request_locks_and_saves`. Manually verified with a throwaway
  `APPDATA`: a second launch forwards and exits in about 90 ms, `--lock` leaves the primary running, `--exit`
  closes it, a lone `--lock` starts nothing, and a different `APPDATA` runs separately. With Command
  hosting, the icon stays hidden. On exit while hidden, tray-icon prints "Error removing system tray icon"
  (debug console only).

1. **Squatting (same-user malware):** pre-creating `Local\Axentra.Vault.<hash>.Instance` makes every launch
   "forward" and exit, so Vault can't start; pre-creating the events lets the squatter see or swallow
   commands; creating `Local\Axentra.TrayHost` hides Vault's icon. These are denial-of-service only (the
   window, auto-lock and Exit still work), but say so in the README or harden the names (explicit DACL,
   check the mutex owner, `Global\`-free private namespace via `CreatePrivateNamespace` + boundary
   descriptor?). Is the FNV name predictable enough to matter?
2. **What the channel can do:** confirm Show, Lock and Exit are the only effects; Exit goes through the
   normal close path (unsaved-changes prompt, recovery guard). Lock skips the unsaved-changes prompt and
   auto-saves like idle lock, and ignores `key_job_pending` (a key job's result is dropped with the session;
   confirm no file write can follow). `ASFW_ANY` lets *any* process take the foreground once; is
   `AllowSetForegroundWindow(<primary pid>)` possible without opening the hardened process?
3. **Ordering:** the instance check runs before hardening. Confirm nothing in `start`/`signal` leaks a handle
   or runs long enough to matter, and that the listener thread and the global tray handlers never touch vault
   data.
4. **Tray content:** confirm the tooltip and menu never include entry or file names, and that WDA on the
   tray-icon hidden window and the menu popup is harmless.
5. **Two builds:** the dev "(Test)" copy and the installed release share one instance namespace under the
   real `APPDATA`; launching one while the other runs shows the other. Acceptable?

### B5. Leads we found while preparing this brief
Confirm each lead and file it as a new finding, or reject it.

| Lead | Where | Status |
|---|---|---|
| The Ctrl+C "copy password" shortcut does nothing in the real app. egui-winit 0.36.2 (`lib.rs` ~L1024) turns Ctrl+C into `Event::Copy` and returns without a `Key` event, so `key_pressed(Key::C)` never fires. Both layouts still advertise Ctrl+C in hover text (`session.rs` ~L1386, `floating.rs` ~L1447). kittest sends `Key` events directly, so tests can't catch this. If a label is selected, Ctrl+C sends an unprotected `CopyText`. | `session.rs` ~L483 | Source-verified; confirm on the desktop. Fold into M6. |
| A panicking key, unlock or breach job leaves its slot set (idle lock off, UI stuck). Icon worker panics are swallowed by `fetch_all` (`siteicons.rs` ~L487): the icon job finishes, but that host is silently dropped and not counted as failed (`apply_icon_results` counts only `Err`). The icon job sticks only on a hang (H6.2). | `jobs.rs` ~L36, `session.rs` ~L448, `siteicons.rs::fetch_all` | Source-verified. Fold into L1. |
| SVGZ decompression bomb. | `siteicons.rs::to_png` | Source-verified in usvg; reproduce. |
| Parent-domain fallback requests public suffixes. | `siteicons.rs::fetch` | Source-verified. |
| Breach body isn't capped after decompression. | `breach.rs` ~L82 | Source-verified. Fold into M1. |
| `custom:Password` / OTP columns. | `columns.rs`, `session.rs::row` | Source-verified. |
| Tests can write the owner's real `%APPDATA%`. | `app.rs` `security_tests` | Source-verified. |
| `header.rs` reads the **first** KDF field (~L149), but keepass 0.15.2 uses the **last** (`kdbx4/parse.rs` ~L206). `P` is truncated with `as u32` (~L103). | `vault-core` `header.rs` | Source-verified. Matters for M4's KDF check. |
| Test fixtures are modelled on the owner's real vault: `categorize.rs` `real_world_entries`, plus personal usernames in tests. They ship in the GPL source zip. | `categorize.rs` ~L1192, tests | Privacy. The owner decides. Recommend neutral data. |

---

## 6. Part C: fix the remaining findings

### Status at HEAD (verify each)

| ID | Status | Evidence at HEAD | Changed by |
|---|---|---|---|
| M1 | Partial | HTTP `Content-Encoding` is capped for icons (`read_capped` in `get_bytes`). SVGZ decoded inside `usvg::Tree::from_data` is still unbounded (H6.1), and `breach.rs` ~L82 isn't capped. | `e1fbc66`; new instance in `cc3ca44` |
| M2 | Partial | **Done:** `write_new_backup` uses `create_new` + `sync_all` + read-back.<br>**Still open:** `write_ring` uses a fixed `.tmp` (`File::create`), an unsynced `fs::copy` to `.old` and a plain `fs::rename`; `vault.rs` ~L217 makes `.bak` with an unsynced `fs::copy`; the backup path isn't checked against the database's own files; nothing recovers on restart. | `9a6101f` |
| M3 | Partial | **Done:** missing metadata now counts as a conflict.<br>**Still open:** `open` reads the file before stamping it; `save` checks once, then does slow work; fixed `.tmp`; no re-check before `.bak` and the rename; the stamp is only length + mtime.<br>**Also evaluate** `changed_on_disk`'s `_ => true` (`vault.rs` ~L166): a stamp failure right after a committed save (~L223) leaves `on_disk` at `None`, so every later save prompts Overwrite and every idle lock with unsaved changes goes to memory-only recovery; and when the file was deleted or moved, the Overwrite text ("Another device or app saved…", "kept as .bak", `session.rs` ~L1938) is wrong because no `.bak` is made. Should the dialog say which case applies? | `f7e87b9` |
| M4 | Open | No size caps on whole-file reads: key file (`jobs.rs` ~L82), sidecar (`jobs.rs` ~L102, and `session.rs` ~L2095 `read_ring` on the UI thread at every unlock and key change), KDBX (`vault.rs` ~L92, ~L192). `Envelope::parse`/`KeyRing::parse` have no field, count or duplicate limits, and there's no KDF budget. `envelope.rs` is unchanged since `a264271`. | none |
| M5 | Open | `keyfile.rs` is unchanged since `a264271`. Add-key doesn't verify that the wrapped key opens the database. | none |
| M6 | Partial | **Done:** the masked editor drops Copy/Cut.<br>**Still open:** revealed detail labels are selectable (`selectable_labels` is still the default, true) and copy as a plain `CopyText`; `clipboard.rs` is unchanged (null owner, marker failures ignored, a failed clear isn't retried, no clear on lock); the dead Ctrl+C shortcut (B5). | `81ed196` (editor only) |
| M7 | Open, slightly wider | **Wider:** `rotation_job` was added to the idle-lock suppression.<br>**Still open:** blocking rfd dialogs (`session.rs` ~L2184, ~L2239, ~L2353, ~L2364); editor drafts aren't dirty; jobs can't be cancelled; WebAuthn calls pass no cancellation id. | `9a6101f` (added `rotation_job` to the idle-lock suppression); otherwise none |
| L1 | Partial | **Done:** score saturates.<br>**Still open:** a disconnected `Job` is still reported as running, and since `c8aee0b` this can disable idle lock. | `e1fbc66` |
| I1 | Open | rustybuzz 0.20.1 and ttf-parser 0.25.1 are in `Cargo.lock`. No `deny.toml`. | none |
| I2 | Partial | **Done:** license metadata, LICENSE, installers, README fix.<br>**Still open:** no signing, no third-party notices, no upgrade test, no `--locked` or clean-tree check (Part D). | `c8aee0b`, `5184a15` |

### Remediation specs (one commit each, in this order, each with a regression test)

Keep each change small and contained. In the commit message and in the report, explain any behaviour
change the user will notice.

1. **L1: report dead jobs as failures (do this first; `c8aee0b` made it worse).**
   - **Change:**
     - `Job::poll` returns a terminal failure state on `Disconnected`, e.g. `Running | Done(T) | Failed`. Optionally add `catch_unwind` in `Job::spawn`.
     - Every caller clears its slot and sets a generic error: the unlock job (`app.rs`), `poll_passkey_job`, `poll_icon_job` (defence in depth) and `poll_breach_job`.
     - `fetch_all` reports a host whose worker panicked as a failure instead of dropping it.
   - **Test:** a panicking `Job` reports failure. A session whose `passkey_job` panics gets
     `key_job_pending() == false` and locks after `expire_idle_timer`.
   - **Pitfalls:** never show the panic payload in the UI. Tests must wait with a timeout, not spin.
2. **M6: clipboard.**
   - **Change:**
     - Turn off label selection (`selectable_labels = false` in the theme, or `Label::selectable(false)` at the secret label sites).
     - When not typing and no editor is open, consume `Event::Copy` in `frame_logic` and copy the selected entry's password through `copy_logged`. This also fixes the dead Ctrl+C.
     - Make `PendingClear::clear` return the pending clear on failure, retry it for about 10 s, then show an error.
     - Clear the clipboard on lock.
     - Pass the main window as the `OpenClipboard` owner.
     - If any of the three markers fails, empty the clipboard and report an error.
   - **Tests (kittest):**
     - Reveal a password, drag-select its label and send `Event::Copy`. Assert there's no `CopyText` containing the canary and that a protected copy was recorded.
     - Copy, then lock: the pending clear is consumed.
   - **Pitfalls:**
     - egui copies a label's selection in an end-of-pass plugin, so filtering output inside `ui` doesn't work.
     - Keep normal copy in focused TextEdits (search, title, username, URL).
3. **M7: lock contract.**
   - **Change:**
     - Cap how long a key job can hold off idle lock: about 270 s for registration, and keys × 120 s + 30 s for rotation. Drop the job on lock.
     - Add an `Arc<AtomicBool>` cancel flag that is checked between steps of registration and rotation.
     - Replace the `rfd::MessageDialog` confirmations with `egui::Modal`, as `confirm_dialog` already does.
     - For the save-file dialog (~L2353), either run `AsyncFileDialog` in a `Job` or document the limit.
     - Add `Editor::is_changed()`. Lock and close prompt for a changed draft. Idle lock applies the draft through `edit_with_history`, then goes through the existing save-or-seal path and adds a `lock_notice`.
     - Update `last_activity` from inside the editor viewport.
   - **Tests:**
     - A key job that never finishes still locks once its ceiling passes.
     - A changed draft makes `lock_clicked()` prompt.
     - After an idle lock, the draft is present when the vault is reopened.
   - **Pitfalls:** a worker result that arrives after lock must never be committed. Don't prompt when the
     editor is unchanged.
4. **M5: make `raw_key` match KeePass.**
   - **Change:**
     - Follow keepass 0.15.2 `parse_keyfile` order exactly: real XML parsing (v1 base64, v2 hex; accept only 32 bytes), then exactly 32 raw bytes, then exactly 64 hex bytes with no trimming. Anything else returns `None`.
     - Before registering a passkey for an existing key, verify in the worker that the database opens with password + key (e.g. `Vault::verify_credentials`), so no orphan credential is created.
     - Refactor to `register_passkey_with(register, evaluate)` so tests can inject both.
   - **Test (differential table):** inputs are 32 raw bytes, 64 hex, hex+LF, hex+CRLF, BOM+hex, v1 XML,
     `build_v2` output, v2 with extra whitespace, a wrong root element, a `<DataX>` decoy, and Version only
     inside a comment.
     - For each input, whenever `raw_key` returns `Some(k)`, the database must open with password + `k`.
     - The 65-byte hex+LF file returns `None`.
     - When verification fails, the injected register function is never called.
5. **M4: size and KDF limits.**
   - **Change:**
     - Capped reads (`take(max+1)`): key file 16 MiB, sidecar 64 KiB, KDBX 512 MiB. The sidecar cap applies to
       both `jobs.rs` ~L102 and `session.rs` ~L2095 (`read_ring`), ideally through one shared helper in vault-core.
     - `Envelope::parse` rejects duplicate fields, a credentialId outside 1–1023 bytes (check the base64 length first), a ciphertext that isn't exactly 48 bytes, and an rpId that isn't ASCII or is over 253 characters.
     - `KeyRing::parse` allows at most 32 keys, requires unique IDs, and caps labels at 128 characters.
     - `unlock_with_passkey` requires `rp_id == RP_ID` instead of trusting `keys[0]`.
     - Run a KDF budget check on the same buffer, before `Database::parse`. Defaults: Argon2 memory ≤ 1 GiB, iterations ≤ 100, parallelism ≤ 64; AES rounds ≤ 100M. It returns `VaultError::KdfOverBudget` and needs an explicit user override in the unlock UI. Apply the same check to the re-key parse in `save_overwriting`.
   - **Test:**
     - Each over-limit input is rejected.
     - A header patched to 64 GiB fails fast with `KdfOverBudget`, not `Open`.
     - An unknown KDF fails closed.
   - **Pitfalls:**
     - Reject duplicate KDF fields, because `header.rs` takes the first and keepass takes the last.
     - Use a checked conversion for `P`.
     - Never read the file twice.
     - Keep the key-file cap generous: KeePass accepts any file.
6. **M3: save race.**
   - **Change:**
     - Stamp before and after the read in `open`.
     - Add a SHA-256 of the file to `FileStamp`.
     - Split `save` into prepare (serialize and verify) and commit, and re-check immediately before `.bak` and the rename.
     - Use unique `create_new` temp names and remove them on error.
   - **Test:**
     - An outside writer rewrites the file between prepare and commit, keeping the same length and restoring mtime. Commit returns `ChangedOnDisk`, the outside bytes are intact, and no temp file is left behind.
     - With two `Vault`s on one path, the second save gets `ChangedOnDisk`.
   - **Pitfalls:** keep the H2/H3 behaviour (`previous_credentials`, `on_disk` after commit). Document the gap
     that remains before `MoveFileEx`.
7. **M2: durable key change.**
   - **Change:**
     - Reject a backup path that matches any of the database's own files: db, temp files, `.bak`, `.bak.tmp`, sidecar, `.axvault.tmp`, `.axvault.old`. Compare case-insensitively after canonicalizing the parent folder, and keep the list in one place in vault-core.
     - In `write_ring`, use a unique `create_new` temp, write `.old` as temp + sync + rename, and do the final rename with `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`. Do the same for `.bak`.
     - On restart, if passkey unlock fails, try envelopes from `.axvault.tmp` and then `.axvault.old`, accepting one only if its key opens the database.
   - **Test:**
     - Simulated crash: the database is on K2, the sidecar holds K1, and `.tmp` holds K2. Unlock with an injected PRF succeeds and repairs the sidecar.
     - A backup path of `<db>.bak` is rejected without touching any file.
   - **Pitfall:** don't claim power-loss safety without testing it.
8. **M1 remainder.** Move `read_capped` to a shared module and use it in `breach.rs` with a cap of about 2 MB.
   Test with a loopback gzip-bomb server and an injectable base URL.
9. **I1: unmaintained font crates.** A disposition is acceptable. Don't churn the egui stack.
   - **Option:** `resvg = { version = "0.45", default-features = false }` in `app/Cargo.toml`. egui_extras
     already depends on resvg with defaults off, so this may remove rustybuzz from the Windows build.
   - **Acceptance:**
     - `cargo tree -e normal --target x86_64-pc-windows-msvc -i rustybuzz` (and `-i ttf-parser`) finds nothing.
     - An SVG containing `<text>` still goes through `to_png` without an error.
   - **Lockfile:** regenerate `Cargo.lock` once, for this change only.
   - **Otherwise:** record the disposition. Optionally add `deny.toml` and a dependency-check script.
10. **I2:** see Part D.

**New findings you confirm:** fix them with the same rules if they are High or Medium, or if the fix is
small. Examples: the SVGZ bomb (reject gzip magic, or decompress with a cap and call `Tree::from_str`),
the parent-domain fallback (use `site_of`), `custom:Password` and OTP columns, test `APPDATA` isolation,
and the `MaskedBuffer::type_id` marker. Otherwise recommend a fix.

---

## 7. Part D: release readiness

**Signing:** nothing is signed. `Get-AuthenticodeSignature` reports `NotSigned` for the release exe, the
Setup.exe and the MSI, and Inno's `unins000.exe` is unsigned too. Smart App Control is off on this PC, which
is why the unsigned builds ran. The README says Smart App Control blocks unsigned release builds, and
SmartScreen will warn as well. Signing is the owner's step. Don't sign anything; list what must be signed
(the exe before packaging, the Setup.exe, the uninstaller via Inno `SignTool`/`SignedUninstaller`, and the
MSI) and anything that blocks it.

Check these and report each one as pass, fail or not tested:

1. **Per-user install folder.** Both installers install to `%LOCALAPPDATA%\Programs\Axentra Vault`, which the
   user can write to. Same-user malware, the threat `hardening.rs` targets, can do all of the following
   without touching the running process:
   - replace the exe;
   - plant a DLL that is a load-time import but not a KnownDLL (for example `webauthn`, `wscapi`, `dwmapi`, `uxtheme`, `opengl32`, `bcrypt` or `VCRUNTIME140`), which loads before `main()`;
   - edit the shortcuts.

   Weigh a per-machine option, `/DEPENDENTLOADFLAG:0x800` and `+crt-static`. Confirm the search order
   with Procmon.
2. **Runtime and OS reach:**
   - `VCRUNTIME140.dll` is imported, but no installer bundles or checks for it. Test on a clean VM, or use a static CRT.
   - `webauthn.dll` is a load-time import, while Inno has `MinVersion=10.0.17763` and the MSI has no OS check. Confirm which Windows build first ships the API.
   - `x64compatible` also allows ARM64.
3. **Inno and MSI together:**
   - **Shared identity:** they use different identities (AppId vs UpgradeCode) but the same folder and the same Start Menu shortcut name. In a VM, test both install orders: two Add/Remove entries? Does uninstalling one break the other?
   - **Inno:** has no downgrade guard.
   - **MSI:** doesn't allow same-version upgrades, so two different 0.1.0 builds could install side by side. Its `RemoveFolder` on the shared `%LOCALAPPDATA%\Programs` should run only when the folder is empty.
4. **Version sources.** The installers take their version from `[workspace.package]`. The exe's
   FileVersion comes from the app crate's own hard-coded `version`. The two can drift.
5. **Upgrading while the app runs.** Inno has `CloseApplications=yes`, but the app cancels close when the
   vault is dirty and while recovery is pending. There's no AppMutex. MSI behaviour with the exe in use
   under a per-user install is unknown. Test in a VM.
6. **Uninstall** leaves `%APPDATA%\Axentra Vault`, including vault paths in `last-database.txt` and
   `unlock-times.txt`. Document this.
7. **`package.ps1`:**
   - It runs without `--locked` and has no dirty-tree check. The exe is built from the working tree while the zip comes from `HEAD`, so a dirty tree breaks GPL Corresponding Source.
   - `SHA256SUMS.txt` is unsigned and doesn't cover the bare exe.
   - The default ISCC path is user-writable.
   - It passes `-acceptEula wix7`.
8. **Binary hygiene:**
   - The release exe contains about 680 copies of `C:\Users\shawn\.cargo\registry\…` (panic locations). Consider `--remap-path-prefix`.
   - There's no application manifest, no Control Flow Guard and no CET-compat flag.
   - The release build has no console and unwinds, so a panic on the main thread exits silently.
   - Run `cargo test --release -p vault-core` once (release builds have overflow checks off).
9. **Notices and links:**
   - There's no third-party notices file (crate licenses, egui's bundled fonts), and dependency licenses haven't been checked for GPL-3.0 compatibility.
   - `AppPublisherURL` and `ARPURLINFOABOUT` point to `MECMwithShawn/Axentra-Software`, which is private, so the link is dead for users.
   - The source zip includes `docs/audit-evidence/`, `spikes/`, the plugin, and the owner-derived test fixtures (B5).
10. **User-facing security claims.** Give each claim a verdict: accurate, needs a qualifier, or false.

    | Claim | Where | May be contradicted by |
    |---|---|---|
    | "can't read the vault's memory or inject code into it"; "can't read or write … create threads … duplicate its handles"; "same approach KeePassXC uses" | `floating.rs` ~L2255; README "Against other programs" | B1.2, B1.3 (threads, token), B1.6, D1 (DLL planting) |
    | "Auto-lock after 5 minutes without input … still locks" | `floating.rs` ~L2256; README Auto-lock | H2 Q1/Q2, M7, L1 |
    | "cleared after 12 seconds … kept out of clipboard history and cloud clipboard" | `floating.rs` ~L2257; README Clipboard | M6, B5 Ctrl+C lead |
    | "stay masked until you choose Show" | `floating.rs` ~L2258 | B3.1–B3.3 (`custom:Password`, OTP, Notes column) |
    | "Recent Activity … never includes secret values" | `floating.rs` ~L2259 | check every `log` call |
    | Icons "straight from that site … only the site's address is used" | `floating.rs` ~L2267 | H5.1 (parent-domain fallback) |
    | "Screen-capture malware … see a blank window"; README Screen capture | `floating.rs` ~L2238, `session.rs` ~L609 | B1.6 (`SendInput`, UIA), B1.7, B1.8 (`capture.txt`) |
    | "Secret fields keep no undo history" | README | H1 Q1–Q3 |
    | "your passwords never leave this PC" | breach dialog, `session.rs` ~L2187 | B2.3, B2.5 |
    | "Your vault files and preferences … are never touched by installing, upgrading or uninstalling" | `AxentraVault.iss` `FinishedLabel` (~L60) | D3, D6 |

**Before publishing `vault-v0.1.0`, all of these must be true** (confirm or amend this list in your verdict):
- No open Critical or High. Every Medium is fixed or has a written disposition the owner accepts.
- The README and the in-app security text claim only what you verified.
- Signed exe, Setup.exe, uninstaller and MSI, with a timestamp.
- Install, upgrade and uninstall tested on a clean VM, including the VC++ runtime question and running the
  app with Smart App Control on.
- `package.ps1` builds with `--locked` from a clean tree; checksums cover every published file.
- A third-party notices file ships in both installers.
- A fresh RustSec advisory and license scan of `Cargo.lock` at the release commit (including the new `sha1`
  and `flate2` entries), with results recorded in `docs/audit-evidence/`.
- Tests pass and `clippy -D warnings` is clean at the release commit. KeePass interop passes.

---

## 8. Constraints

- **Keep KDBX compatibility.** The master password plus the backup `.keyx` must keep opening the database in
  KeePass and KeePassXC. Don't introduce custom ciphers or formats. If a fix changes an on-disk format, say
  so explicitly. KeePassXC isn't installed; don't install it. Report KeePassXC compatibility as Not tested, or
  check it against the KDBX spec only.
- **No new network calls or telemetry in the app.** Live network use during the audit is limited to the
  ignored tests in §3, advisory databases and public documentation.
- **Never use the owner's real data or install:**
  - the owner's real vault and anything under OneDrive;
  - the installed app at `%LOCALAPPDATA%\Programs\Axentra Vault`;
  - the owner's running instance;
  - the owner's `%APPDATA%\Axentra Vault`;
  - the owner's Documents folder (it holds the owner's real backup key file) and any
    `*.keyx` outside `$sb`. The backup save dialog of every key change opens in Documents
    (`session.rs` ~L2350); always browse to `$sb\vault` before saving;
  - the owner's authenticators. Registration always creates a discoverable credential for RP ID
    `localhost`, the same as the real vault's passkey. Don't register on the owner's phone, YubiKey or
    Windows Hello; use a test authenticator the owner provides, or report physical-authenticator items as
    Not reviewed. Never delete credentials from any authenticator.

  Use a throwaway `APPDATA` for every test and app launch. Get test databases from `make_demo` or the
  test helpers.
- **Keep the owner's details out of committed files.** Everything committed ships in the GPL source zip
  (`git archive HEAD`). Don't write the owner's real paths, user name, vault or key-file names, or anything
  else personal into `AUDIT_REPORT_2.md`, commit messages, tests or fixtures. Say "the owner's vault" or
  "the owner's backup key file". This brief names them only so you can avoid them, and it stays untracked.
- **Don't publish, sign, push, upload, install or tag anything.** Don't run `deploy-test.ps1`.
- **Don't delete data.** Back up `dist\` before running `package.ps1`.
- **Commits:** one finding per commit on `audit/fixes`, each with a regression test where practical. Don't
  rewrite or restyle working code, and don't run `cargo fmt` across the workspace; format only what you
  change. After every commit, `cargo clippy --workspace --all-targets -- -D warnings` must be clean and
  `cargo test --workspace` must pass.
- **Dependencies:** no dependency upgrades except the single I1 lockfile change, if you make it.

## 9. Deliverable

**`docs/AUDIT_REPORT_2.md`**, committed as its own docs commit. Use the same per-finding format as your
first report (`docs/AUDIT_REPORT.md`):

```
### [SEVERITY] ID — Short title
- Where: file:line (and function)
- What: one-paragraph description
- Exploit / failure scenario: concrete steps or inputs
- Fix: recommended change (and commit hash on audit/fixes if you made one)
- Remediation: (optional) what was implemented on audit/fixes, with the commit hash
- Confidence: confirmed (reproduced/tested) | likely | speculative
```

Severities are Critical / High / Medium / Low / Info, ordered most severe first. Give new findings new IDs
(e.g. N1, N2…) so they don't collide with H/M/L/I.

**Sections:**
1. **Validation:** exact commands, test counts, interop result and tool versions.
2. **Re-verification of H1–H6:** for each one: still fixed / regressed / partially fixed, with your answers to the Part A questions.
3. **New findings:** from Part B and §5 B5, including those you rejected and why.
4. **Remaining first-pass findings:** M1–M7, L1, I1, I2, each with final status, commit hash and behaviour change.
5. **Looks good:** what you checked and found sound.
6. **Not reviewed:** what you didn't get to, including anything that needs a physical authenticator, a VM or a signed build.
7. **Release-readiness verdict for v0.1.0:** ship / ship after fixes (list them) / don't ship, plus your amended version of the Part D "must be true" list.

**Commits on `audit/fixes`:** one per finding, with the report updated to the final hashes.
