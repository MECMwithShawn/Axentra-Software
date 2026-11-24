# Axentra CloudDrive: Audit and Plan

Location: `docs/clouddrive/AUDIT-AND-PLAN.md`
Source root: `OneDrive/source` in this repository. All file:line references below are relative to `source/` unless marked otherwise.
Baseline as of 2026-10-29: `cargo build --workspace --locked` gives 0 warnings. `cargo test --workspace --locked` gives **172 passed, 0 failed**: cli 4, core unit 42, core e2e 81, graph 5, mock graph_client 14, ui 26. Real FUSE was used; D-Bus was not exercised.

---

## 1. Current architecture assessment

| Layer | What exists | Evidence |
|---|---|---|
| Graph client | Crate `axentra-onedrive-graph`. The `Graph` struct is concrete and has **no trait**. Every call hard-codes `/me/drive/items/{id}`. | crates/graph/src/client.rs:339, 356, 462-1108 |
| Auth | Public client using auth code + PKCE (loopback `127.0.0.1:0`) or device code. Tokens are a plaintext JSON file written 0600 atomically. There is no keyring. | auth.rs:23, 117, 247-264, 285-354, 373-412 |
| Retry | 6 attempts with 500ms·2^n backoff. Retries on 429/5xx and honours Retry-After (capped at 300 s). One forced refresh on a 401. Upload stall cut-off is 120 s. | client.rs:28, 155-204, 403-442 |
| Engine | `Engine` holds `pub graph: Graph` (concrete). The loop runs push → pull(delta) → push, aborts on pause or sign-out, and makes one DB transaction per change. | core/src/engine.rs:188, 239, 826-927 |
| Local changes | Detected through FUSE only. There is no scanner and no inotify. | engine.rs:513-543; fs.rs:231, 269, 318-329, 462, 687-709, 932 |
| Downloads | On demand into `cache_dir/{ino}`. QuickXor is verified **in the engine**, not in `Graph::download`. | engine.rs:442, 2374-2490, 2434-2452; client.rs:565-586 |
| Conflicts | Conflicting copies are kept as `name-HOSTNAME`. Server responses 409, 412 and 404 are each handled. There is **no mass-delete guard**: the warning only appears after 50 or more deletions have been applied. | engine.rs:1157-1175, 1309-1317, 1538-1631, 2196-2312 |
| State DB | `~/.local/share/axentra/onedrive/state.sqlite` in WAL mode. All access goes through one `Mutex<Connection>`. There is no `user_version`. Columns are added ad hoc with `pragma_table_info`. | db.rs:133-177, 236-268; config.rs:202 |
| Daemon | One process runs one Engine and one FUSE mount. The D-Bus name is claimed before the mount, which enforces a single instance. Shutdown waits 10 s for sync, then unmounts with a 5 s limit. | daemon.rs:28-119; ipc.rs:292-298 |
| D-Bus | `io.github.mecmwithshawn.AxentraOneDrive` / `…/AxentraOneDrive` / `…AxentraOneDrive1`. 20 methods and 2 signals. | config.rs:8-22; ipc.rs:118-368 |
| UI | Slint tray, flyout and settings. One proxy and one Account. Settings pages are numbered 0-5 in a non-monotonic order. | crates/ui/src/app.rs:27-35, 381; settings.slint:330-400 |
| CLI | `axentra-onedrive` built with clap. Most subcommands are thin D-Bus calls. | crates/cli/src/main.rs:26-125, 282-291 |
| Tests | The mock is a real axum HTTP server that speaks the Graph wire protocol, with fault injection. The only test seam is the base URL. | crates/mock/src/lib.rs:49-91, 355-382, 589-701; core/tests/e2e.rs:43-80 |
| Packaging | **There is no OneDrive .deb.** Install is per-user via `install.sh`, which builds from source into `~/.local/share/axentra/apps/onedrive` and uses user systemd units. | install.sh:14-149 |

Overall: the sync path is mature and well tested, but it is tied to Graph at every layer. Adding Proton as a second sync engine inside `Engine` would put the 81 e2e tests at risk. Proton should come in as a **separate mirror component** behind new, narrow traits.

## 2. Reusable components

- `RateLimiter` (graph/throttle.rs:10-49) can be used for any provider as it is.
- Retry and stall helpers `backoff`, `retryable`, `retry_after` and `send_watched` (client.rs:155-204) can move to a shared module.
- The atomic 0600 file-write pattern (auth.rs:247-264), with a fix for permissions on a pre-existing `.tmp` file.
- Error classification `is_offline`, `is_transient` and `is_not_found` (client.rs:86-112) gives the shape for a provider-neutral error enum.
- `DeltaPage{items,next_link,delta_link}` (client.rs:207-211) fits as the source change-feed shape.
- `QuickXorHash::of_file` (quickxor.rs) verifies the OneDrive source side.
- `Stamp`/`Changed` detects a file that changes mid-upload (client.rs:313-337). This is local logic.
- Db and Store wrapper plus the `CREATE IF NOT EXISTS` and `pragma_table_info` pattern (db.rs:239-268). Use it in a **separate** mirror DB file.
- `Backoff` shape (engine.rs:104-130). The mirror needs a *persisted* equivalent.
- `TransferGuard` and `progress` (engine.rs:132-143, 571-600), and the activity table (`kind` is free text).
- The stop-watch and pass-mutex pattern of `cycle()` (engine.rs:850-892) is the model for a mirror worker.
- `Paths::under(root)` (config.rs:208-216). Config uses `serde(default)`, so adding a `[mirror]` or `[proton]` table stays backward compatible (config.rs:31-63, test at :242-244).
- `Notifier` (notify.rs:176-221), `IgnoreRules` (names.rs:71-90) and `status::human_size`.
- UI pieces:
  - Theme, `FluentButton`, `InfoBar` and `Meter` (theme.slint:6-382).
  - `Card`, `OptionCard`, `NavItem` and `ConfirmPopup` (settings.slint:8-268).
  - The `Cmd` enum with `spawn_call` (app.rs:37-74, 972-990).
  - The Snapshot refresh pattern (app.rs:911-970).
  - `open_terminal` (app.rs:437-483).
  - `--screenshot settings-N` (shot.rs:137-190).
- Mock skeleton (crates/mock/src/lib.rs:341-383): request log, `requests(needle)` and fault knobs. Reuse it to prove the mirror never issues destructive calls.

## 3. OneDrive-specific coupling to extract

Extraction here means *wrapping behind adapters*, not refactoring the engine.

- The engine owns a concrete `Graph` and uses about 30 call sites. The types `DriveItem`, `UploadTarget`, `GraphError`, `QuickXorHash` and `TokenStore` run through the conflict logic (engine.rs:16-18, 188, 239, 361). **Leave these in place in M1.** Add a read-only `SourceProvider` adapter around `Graph` instead.
- Graph-specific details:
  - Delta semantics: nextLink and deltaLink, 410 resync, and the package and remoteItem facets (engine.rs:937-1068, 1203-1220).
  - `qxh`, `ctag` and `etag` columns, If-Match, and 409, 412 and 404 contracts (db.rs:142; engine.rs:2133-2312).
  - Microsoft identity: `AuthConfig`, Entra scopes, and the `client_id`, `tenant`, `authority` and `graph_base` config keys (auth.rs:23-47; config.rs:34-39, 70-71, 144-150).
- OneDrive-only features: share links, versions, thumbnails, web URL, recycle bin and manage-storage URLs (client.rs:589-624, 1085-1110; ipc.rs:181-222; app.rs:1093-1104, 1164, 1176-1181).
- Account shape: `drive_type` personal or business, `title()` returning "OneDrive - Personal", and statfs quota (status.rs:268-286; fs.rs:1017-1035).
- Name rules (names.rs:3-62) are OneDrive and SharePoint rules. **Do not apply them to Proton.**
- Packaging text and links:
  - Command card tagline and links (extras/command/onedrive.json:4, 19-22).
  - The .desktop Comment and Keywords.
  - The `client_id` setup check in `install.sh:147`, which needs to become per-provider.

## 4. Proton Drive integration options

| Option | Official | Licence | Auth (who handles the password) | Structured output | Integrity | Fit with brief | Verdict |
|---|---|---|---|---|---|---|---|
| **Proton Drive CLI `proton-drive`** (ProtonDriveApps/sdk `cli/`, v0.9.0, 2026-10-05), run as a subprocess | **Yes** | Source MIT. Licence for the release binaries is not stated. | Browser session fork. SRP, 2FA and CAPTCHA happen in the browser. The session is stored in libsecret `ch.proton.drive/drive-sdk-cli`, or in pass. | `--json`/`-j` gives single-line JSON with an end-of-transfer summary and a `failures` array. There is no progress stream and no documented schema. | SDK crypto. Downloads verify block hashes and the manifest signature, then SHA-1, and raise `IntegrityError`. Uploads store the plaintext SHA-1 in encrypted xattrs. | No custom crypto, no shell, no secrets on argv. | **RECOMMENDED** |
| Official SDK (TypeScript or C#) through the C ABI `proton_drive_sdk.h` + protobuf | Yes | MIT | **Not included.** Login would need the incubating account module, which "will not be promoted", or our own SRP. | Typed and protobuf | Same as the first-party apps | Login is unsolved. NativeAOT on Linux is untested. "Not yet ready for third-party production use". | Upgrade path after GA and after an Account SDK exists |
| rclone `protondrive` | No | MIT | Username, password and TOTP secret. The salted key passphrase is stored in rclone.conf. | `lsjson`, rc API | Community Go crypto | Reverse-engineered. Uses a deprecated endpoint under a temporary integrity exemption. v1.75.0 had a format regression. Abuse blocks have been reported. | Rejected; at most an opt-in fallback, decided by the user |
| narrrl/proton-sdk-rs, tirbofish/proton-sdk-rs2 | No | MIT (licence on narrrl is NOASSERTION) | In-process SRP | Rust API | Reimplemented Proton crypto | Breaks the "never custom Proton crypto" rule | Rejected |
| DonnieDice/protondrive-linux | No | AGPL (unverified) | Custom SRP plus an API proxy | None | Web app | Proxying conflicts with Proton policy | Rejected |
| henrybear327 Proton-API-Bridge | No | MIT | Password, no 2FA | Go | Community | Archived 2026-02-05 | Rejected |

**Recommended path:** an official Linux CLI with structured output **does exist**. Wrap the pinned official `proton-drive` CLI v0.9.0 behind a `MirrorDestination` trait.

- Call it with `std::process::Command` and an argument vector. Never use `sh -c`.
- Scrub the environment. **Refuse to run** if `PROTON_DRIVE_CREDENTIALS_STORE=unsafe_file`.
- Pass no credentials on argv or through env. The app never sees the password, the TOTP secret, or key material.
- Allowlist these subcommands only: `auth login`, `auth logout`, `list`, `info`, `create-folder`, `upload`, `download`, `version`.
  - **Never** call `trash`, `delete`, `empty-trash`, `move`, `rename` or `restore`.
  - **Never** use `--file-conflict-strategy replace`, because it trashes the remote file.
- Always pass explicit `--file-conflict-strategy` and `--folder-conflict-strategy` flags. Under `--json`, a prompt turns into an error.
- Run one invocation at a time and only on demand. Never run timed recursive listings of Proton.
- The user installs the binary from proton.me/download/drive/cli. The app checks it against a pinned SHA-512.
- Sources: https://proton.me/blog/proton-drive-cli · https://github.com/ProtonDriveApps/sdk/blob/main/cli/README.md · https://github.com/ProtonDriveApps/sdk/blob/main/README.md

**Verified, from primary sources:**
- The CLI exists for linux/x64 and is at v0.9.0.
- It supports `--json`, browser login and libsecret storage.
- Upload conflict strategies are `create-new-revision`, `rename`, `replace` and `skip`. Uploads with identical content (same SHA-1) are skipped.
- Downloads raise `IntegrityError` on a mismatch.
- The SDK retries 429s and enters a 60 s cooldown after 50 consecutive 429s.
- There is no official Linux sync app yet.
- A breaking crypto-model change is planned for late 2026 or early 2027.

**Unknown or unverified:**
- The per-command JSON schema. Only the transfer summary and `{"signInUrl"}` have been seen.
- Exit codes. 0, 1, 2 and 130 are inferred from code.
- Whether `info --json` exposes `claimedDigests.sha1` in a stable field.
- The meaning of `sha1Verified`.
- Exact `upload` destination syntax at v0.9.0.
- Whether interrupted uploads leave drafts behind.
- Whether Proton accepts a third-party GUI driving the unmodified CLI, which identifies itself as `cli-drive`.
- The licence of the binaries.
- Numeric rate limits.
- The exact date of the crypto migration.

## 5. Technical risks and limitations

1. **Proton maturity.** The CLI is pre-1.0 with no schema guarantee. Mitigation: pin the version and its SHA-512, keep a fixture-based contract test per version, and refuse to run an unrecognised version.
2. **Crypto-model break** in late 2026 or early 2027. Older SDK and CLI builds will stop working. Mitigation: make upgrading the CLI a checkpoint, and have the mirror fail closed.
3. **Unsigned binary.** Only a SHA-512 is published. This is a known gap; record it in MIGRATION_READINESS.md.
4. **Policy and identity.** The CLI reports itself as `cli-drive`, and Proton has not said whether a wrapper is acceptable. Telemetry is on by default and follows the account preference. The app must show "This is a third-party application not officially supported by Proton." and must use no Proton branding or logo.
5. **Token race.** A second process refreshing `tokens.json` could race the daemon. Mitigation: run the mirror **inside** the daemon and share `Arc<TokenStore>` (auth.rs:124-240).
6. **DB contention.** There is a single `Mutex<Connection>` (db.rs:236). Mitigation: put the mirror manifest in its **own SQLite file and connection**, and never write to `items`.
7. **Files On-Demand.** Reading through `~/OneDrive` hydrates the cache (fs.rs:4-6, 187-199). Setting the pinned xattr frees up space (fs.rs:1056). Mitigation: the mirror never touches the FUSE mount or xattrs. It downloads through its own `Graph` instance into staging.
8. **Unverified download.** `Graph::download` does not verify the hash and creates `dest` directly (client.rs:565-586). Mitigation: download to a temp file in staging, verify QuickXor, then rename.
9. **ino and id instability.** A 410 resync, `split_conflict` and save-by-rename all change ids (engine.rs:1039-1056, 1541-1596; fs.rs:687-709). Mitigation: the manifest uses its **own** Graph delta cursor, keyed on OneDrive id plus path, and tolerates id changes.
10. **Non-destructive semantics.**
    - Deletes on OneDrive are recorded as `source_deleted` and are never propagated.
    - A rename or move on OneDrive produces a new copy at the new path. The old copy stays, so duplicates build up. This is documented.
    - Changed content is either skipped as `diverged` or added as a new Proton revision. The previous revision is kept. Which one is a checkpoint decision.
11. **Staging bounds.** The CLI uploads from a path, so each file has to be fully on disk. A file larger than the staging cap is `skipped(too_large)` until the user raises the cap. Free space is checked with statvfs before each download.
12. **Rate limits.**
    - Graph: the mirror gets its own `RateLimiter` so it does not throttle user sync.
    - Proton: no published numbers. Run serially, honour 429 and cooldown errors, and add persisted backoff.
13. **Rename hazards.**
    - GNOME `extension.js:11` `PANEL_TITLE` must match `flyout.slint:347`.
    - `icons.rs:19-35` rewrites only `accent` on the Command card, so its name stays stale until reinstall.
    - The hand-made drop-ins in `axentra-onedrive.service.d/` are keyed to the unit name.
14. **Gaps in test coverage.**
    - No test exercises D-Bus.
    - No test asserts Slint strings.
    - The e2e suite needs `/dev/fuse`.
    - e2e.rs:189-200 depends on whether `gio` is on the host.
15. **No mass-delete guard** in the existing OneDrive engine. This is pre-existing and out of scope, but listed so nobody assumes it exists.

## 6. Proposed migration architecture

```
axentra-onedrive daemon (unchanged process, bus name, mount)
 ├─ Engine (UNCHANGED: Graph, FUSE, state.sqlite)
 └─ MirrorWorker (new tokio task, default DISABLED)
     ├─ OneDriveSource: own Graph instance sharing Arc<TokenStore>, own RateLimiters,
     │   own delta cursor; read-only methods only (delta/item/download)
     ├─ Staging: $XDG_CACHE_HOME/axentra/onedrive/mirror-staging (cap, default 2 GiB, 0700)
     ├─ Manifest: $XDG_DATA_HOME/axentra/onedrive/mirror.sqlite (own connection, PRAGMA user_version)
     └─ ProtonCliDestination: MirrorDestination over CliRunner (std::process::Command, argv, env-scrubbed)
```

**New crate `crates/provider`** (package `axentra-clouddrive-provider`) contains:
- `SourceProvider` (read-only): `account`, `change_page(cursor)`, `item`, `download_to(tmp)`.
- `MirrorDestination`: `account`, `list(folder)`, `ensure_folder`, `upload_new`, `upload_revision`, `stat` (claimed sha1 and size), `download_to` (used for round-trip verification).
- **No** `delete`, `move`, `rename` or `trash` methods exist on the trait, so destructive calls cannot be expressed in code.

Implementations:
- `GraphSource`: a thin wrapper over the existing `Graph`. The engine is not changed.
- `ProtonCliDestination`.
- `MemSource` and `MemDestination`: in-memory fakes that log every call.

**Manifest schema (v1):**
- `mirror_items`: `source_id`, `source_path`, `size`, `qxh`, `ctag`, `mtime`, `sha1`, `sha256`, `dest_path`, `dest_uid`, `dest_claimed_sha1`, `state`, `verify_level`, `attempts`, `next_attempt_at`, `last_error`, `first_mirrored_at`, `last_verified_at`.
  - `state` is one of `planned`, `staged`, `uploaded`, `verified`, `skipped`, `diverged`, `failed` or `source_deleted`.
- `mirror_runs`: `id`, `mode` (dry or real), `started`, `finished`, counts and bytes.
- `mirror_kv`: the delta cursor and the destination root.
- This file is **never** part of `Store::reset`, so it survives a OneDrive unlink.

**Verification levels.** Each level is reported exactly as reached, never rounded up.
- `L0 planned`: in the plan, nothing transferred yet.
- `L1 source-verified`: the staged bytes match OneDrive's QuickXor.
- `L2 uploaded`: the CLI exited 0 and its summary reported `failures=[]`.
- `L3 remote-claimed`: the SHA-1 Proton reports for the file matches our local SHA-1. This proves the metadata we wrote, not the stored bytes.
- `L4 round-trip`: the file was downloaded back through the CLI, which ran its signature and `IntegrityError` checks, and the SHA-256 of that download matches the source SHA-256.

**Dry run** (`mirror plan`):
- Walks the source delta and runs Proton `list --json` on the destination root, both read-only.
- Writes a plan report: counts, bytes, conflicts, too-large files and unsupported names.
- Makes **zero** mutating calls. A test asserts this through the fake runner's log.

**Surfaces.** All additions are additive; nothing existing is renamed.
- D-Bus methods: `MirrorStatus()->s`, `MirrorPlan()->s`, `MirrorNow()`, `MirrorSetEnabled(b)`, `ProtonAccount()->s`.
  - The UI treats `UnknownMethod` as "needs update", so an older daemon does not look like "isn't running".
- CLI groups: `proton {login,logout,status}` and `mirror {plan,now,status,enable,disable}`.
- GUI: Settings page 6 "Proton Drive" and page 7 "Cloud Mirror", plus `Tray1.Show("mirror")`.

**Why the mirror runs in the daemon and not a separate process:**
- It shares the token refresh, so there is no `tokens.json` race.
- Pause and sign-out already flow through the stop watch.
- It isolates cleanly: separate DB file, separate limiters, no FUSE use and no `items` writes, and it is off by default.

**Regression guard:** the 172 baseline tests must stay green at every milestone, with no edits to existing e2e assertions.

## 7. Implementation sequence

| M | Deliverables | Acceptance tests |
|---|---|---|
| **M1** Rename + abstraction groundwork (offline) | See §9 | Baseline 172 pass + new tests; 0 warnings; `--screenshot` of flyout and settings shows "Axentra CloudDrive" and the unchanged icon; grep proves all §8 ids unchanged |
| **M2** Manifest + planner + dry run (offline) | `crates/mirror`: manifest with `user_version=1`, planner, `mirror plan` with a JSON and text report; name and size checks; staging-budget calculation | Unit tests for schema create and reopen idempotency; plan against the OneDrive mock + `MemDestination` matches expected counts; dry run makes **0** mutating calls; opening a v2 DB from v1 code fails closed |
| **M3** Proton CLI adapter (offline) | `ProtonCliDestination` + `CliRunner` trait; version and SHA-512 pin check; subcommand allowlist; env scrub and refusal of `unsafe_file`; JSON parsing against fixtures taken from the v0.9.0 source; error mapping covering 429, conflict, `IntegrityError` and unknown output | Fixture tests; a forbidden subcommand fails to compile or is rejected at runtime; argv never contains secrets (property test); unknown JSON leads to a fail-closed error; an `#[ignore]` live contract suite exists but is **not run** |
| **M4** Executor (offline) | Staging with cap and statvfs; QuickXor verification; SHA-1 and SHA-256; serial upload; L2/L3/L4 verification; persisted backoff; crash-resume; cancel on stop | e2e: OneDrive mock → mirror → `MemDestination`. Delete, rename and modify on OneDrive leave all prior destination files intact. Kill mid-run and resume with no duplicates. Staging never exceeds the cap. A bit-flip in the source is caught at L1 and in the destination at L4 |
| **M5** Daemon + D-Bus + CLI + GUI | MirrorWorker task (off by default), additive D-Bus methods, CLI groups, Settings pages 6 and 7, third-party notice before login, `open_terminal` flow for `proton-drive auth login` | Daemon test with mirror disabled behaves exactly like the baseline; enabled with fakes, completes a run; UI tolerates an old daemon (UnknownMethod); screenshots of pages 6 and 7 in light and dark |
| **M6** Packaging + docs | `packaging/linux/package.py` (modelled on Notepad's): /opt/axentra/onedrive, user units in /usr/lib/systemd/user, the same icon name and Command id. Docs: README, LINUX.md, CHANGELOG, `docs/clouddrive/MIGRATION_READINESS.md`, `PROTON_INTEGRATION.md`, `VERIFICATION.md` | Lintian-clean .deb built in scratch; `dpkg -c` shows the expected paths; install tested in a throwaway VM or container **only**; documented coexistence with the per-user install |
| **M7** Live, controlled | First Proton sign-in; live contract suite against a pinned CLI; real dry run; mirror of one test folder to an empty Proton folder, verified to L4 | Each step only after its §10 checkpoint; report attached to MIGRATION_READINESS.md |
| **M8** Production mirror | Full mirror in batches with a progress report; scheduled runs optional (minimum once a day) | Manifest coverage report: % at L3 and % at L4; zero destructive calls in the log |
| **M9** Cutover (optional) | User-chosen plan, for example to stop OneDrive sync | Checkpoint only; nothing automatic |

## 8. Existing functionality and identifiers that must remain unchanged

- **Paths**
  - `~/.config/axentra/onedrive/config.toml`
  - `~/.local/share/axentra/onedrive/{tokens.json,state.sqlite}`
  - `~/.cache/axentra/onedrive/files`
  - App dir `~/.local/share/axentra/apps/onedrive`
  - `~/.local/bin/axentra-onedrive`
  - Mount `~/OneDrive` as the default (config.rs:72, 195-216; install.sh:14, 41-43, 62, 168)
- **Tokens**
  - JSON shape `{access_token, refresh_token, expires_at}`
  - The `tokens-new.json` rename flow
  - `client_id`, `tenant`, `authority` and the redirect `http://localhost:{port}` (auth.rs:50-56, 288; cli main.rs:546-561)
- **Config** TOML keys (config.rs:33-63). `install.sh` seds `icon_color` and greps `client_id`.
- **DB**
  - Tables `items`, `tombstones`, `kv`, `activity` and `thumbs` with their columns and indexes
  - kv keys `delta_link`, `scan_next`, `scan_orphans`, `scan_skipped`, `synced_once`, `drive_id`, `account`, `paused_until`, `last_sync`, `thumbs_mount`
  - ino values
  - `PENDING_PREFIX` `.axentra-onedrive-pending-`
  - Conflict name `name-HOSTNAME`
  - `Store::reset` semantics (db.rs:133-177, 822-828; engine.rs:38)
- **D-Bus**
  - `io.github.mecmwithshawn.AxentraOneDrive`, `/io/github/mecmwithshawn/AxentraOneDrive`, `io.github.mecmwithshawn.AxentraOneDrive1`
  - All existing method and signal names and signatures, and the JSON field names (ipc.rs:118-281; status.rs:227-276)
  - Tray `…AxentraOneDrive.Tray` / `.Tray1` and the `Show` values `panel` and `settings` (app.rs:170-191)
- **xattrs**
  - `user.axentra.onedrive.{status,shared,url,error,pinned,thumbnail}` and the status token values (fs.rs:38-47; db.rs:52-64; thumbs.rs:21)
- **FUSE** FSName and Subtype `axentra-onedrive` (fs.rs:1178-1179).
- **Binaries and units**
  - `axentra-onedrive`, `AxentraOneDrive`, `axentra-onedrive-mock`
  - `axentra-onedrive.service`, `axentra-onedrive-tray.service`, and the user's `axentra-onedrive.service.d/` drop-ins
- **Desktop and shell**
  - `axentra-onedrive.desktop`, `StartupWMClass=axentra-onedrive`
  - `set_xdg_app_id("axentra-onedrive")` (ui main.rs:32)
  - SNI id `axentra-onedrive` (tray.rs:29)
  - GNOME extension UUID `axentra-onedrive@mecmwithshawn.github.io`
  - Command manifest id `onedrive` / `onedrive.json`
- **Icon**
  - Theme name `axentra-onedrive`
  - PNG bytes `extras/icons/apps/ice/*` (= Icon-Kit `onedrive-*.png`), default colour `ice`
  - hicolor paths
  - Emblems `axentra-onedrive-{online,local,pinned,syncing,error,shared}`
  - Notification `app_icon` `axentra-onedrive`
- **Crates and types**
  - `axentra-onedrive-{core,graph,mock}`
  - `Graph`, `GraphError`, `UploadTarget`, `DeltaPage`
  - The Graph wire shapes the mock emulates
  - QuickXor base64 output
- **CLI** subcommands and flags (cli main.rs:38-124). Settings page integers 0-5. New pages are appended as 6 and 7.
- **Strings about the Microsoft service** (for example "OneDrive - Personal", "your OneDrive is at", name-rule messages, the e2e.rs:3062-3065 assertions, the trademark notices). These are not branding.
- **HTTP User-Agent** `AxentraOneDrive/<ver>` (client.rs:358-362). Keep it.

## 9. MILESTONE 1 definition

**Scope:** no network, no install, no change to the live daemon, mount, tokens or DB, and nothing touches Proton. Everything is verified with `cargo test` and `--screenshot`.

**A. Display rename to "Axentra CloudDrive".** Display strings only; every §8 id stays as it is.
- Add a brand constant `axentra_onedrive_core::BRAND = "Axentra CloudDrive"` and a Slint `Theme.app-name` global.
- `crates/ui/ui/flyout.slint:347, 565, 851`; `crates/ui/ui/settings.slint:271, 370, 650`; `crates/ui/ui/theme.slint` (add `app-name`).
- `crates/ui/src/tray.rs:33, 56, 88, 102, 109`; `crates/ui/src/app.rs:473, 478, 720, 823, 841`.
- `crates/ui/src/present.rs:325` (About: "Axentra CloudDrive is an independent, open-source client that syncs with Microsoft OneDrive…", keeping the trademark sentence), `present.rs:352`, and `present.rs:229` (signed-out header).
- `crates/ui/src/icons.rs:19-35`: also sync `"name"` on the Command card. Do not touch the icon files.
- `crates/core/src/notify.rs:206` (`app_name`), `engine.rs:701`, `daemon.rs:50`, `fs.rs:1200`, `config.rs:122` (header), `thumbs.rs:321`.
- `crates/graph/src/auth.rs:269` (browser success page).
- `crates/cli/src/main.rs:30, 39, 164, 287, 349`. Leave `:352`, the Entra app name, as it is.
- `crates/{cli,ui}/Cargo.toml:3` descriptions. Bump the version to 0.2.0 in `Cargo.toml` and `../VERSION`.
- `extras/desktop/axentra-onedrive.desktop:3, 5`; `extras/command/onedrive.json:3-4`; `extras/systemd/axentra-onedrive{,-tray}.service:2`.
- `extras/gnome-shell/axentra-onedrive@mecmwithshawn.github.io/extension.js:11`: match `APP_ID` **and** either title ("Axentra OneDrive" or "Axentra CloudDrive"), so a mix of old and new builds keeps working. `metadata.json:3-4`.
- `extras/nautilus/axentra_onedrive_nautilus.py:1, 11, 136, 153, 358`.
- `install.sh:2, 18, 20, 152, 155, 177`; `uninstall.sh:2, 234, 236`.
- `README.md`, `../README.md`, `../LINUX.md`, `../CHANGELOG.md` (rename entry).

**B. Provider abstraction groundwork.**
- New `crates/provider` containing:
  - Traits `SourceProvider` and `MirrorDestination`, with no destructive methods.
  - A `ProviderError` enum carrying offline, transient, not-found, conflict, rate-limited, integrity, auth-required and other.
  - `ContentDigests{size, qxh?, sha1?, sha256?}` and `VerifyLevel{L0..L4}`.
  - `MemSource` and `MemDestination` with call logs and fault injection.
- `crates/provider/src/graph_source.rs`: a `GraphSource` adapter over the existing `Graph`. It covers delta, item and download-to-temp with QuickXor verification. The engine is not modified.
- `Cargo.toml` workspace members: add `crates/provider`.

**C. Tests.**
- `crates/provider/tests/graph_source.rs`:
  - Runs against the existing `crates/mock`: delta paging, 410 handling, and download verification, including a corrupted byte that must be rejected.
  - Uses the mock's `requests()` to assert that `GraphSource` never issues DELETE, PATCH or PUT.
- `crates/provider/tests/mem_fakes.rs`: covers trait contract behaviour.
- `crates/ui` unit test asserting the presentation strings use `BRAND`.
- A test asserting that the `extension.js` title list contains the `flyout.slint` title.
- `tests/ids_unchanged.sh` (or a Rust test): grep-checks every §8 identifier.

**Exit criteria:**
- `cargo build` gives 0 warnings.
- 172 baseline tests plus the new tests pass.
- Light and dark screenshots of the flyout and settings pages 0-5 are reviewed.
- Nothing is installed and nothing is pushed.

## 10. STOP-AND-ASK CHECKPOINTS

1. **Integration mechanism sign-off.** Confirm the official `proton-drive` CLI v0.9.0 as a subprocess. Accept its known gaps: pre-1.0, no signature, identifies as `cli-drive`, telemetry on by default. Decide whether rclone may ever be an opt-in fallback; the default is no.
2. **CLI acquisition.** The user downloads `proton-drive` and approves the pinned SHA-512 and install path. Any later CLI version upgrade, including the crypto-migration upgrade, is asked again.
3. **Changed-content policy.** When OneDrive content differs from what is already mirrored, choose either `skip` and mark it `diverged`, or `create-new-revision` (the old revision is kept). `replace` is never offered.
4. **Destination layout.** Choose the Proton root folder name and path, the staging cap and location, and the minimum verification level that counts as "migrated" (L3 or L4).
5. **Install M1 over the running per-user install.** This restarts `axentra-onedrive.service` and remounts `~/OneDrive`. On Wayland, a new GNOME extension needs the user to sign out and back in.
6. **First Proton sign-in.** Run `proton-drive auth login` in the user's browser, after the third-party notice is shown. Confirm the session is stored in libsecret.
7. **Live contract test.** Run the read-only CLI commands (`version`, `list`, `info`) against the real account to confirm JSON shapes before any write.
8. **First real dry run.** It reads OneDrive and the Proton listing; the user reviews the plan report.
9. **First controlled migration** of one user-chosen small test folder into an empty Proton folder, followed by review of the L4 results.
10. **Production-wide migration.** Approve batch size and schedule (manual by default). Each scale-up step is approved separately.
11. **Any schema change** to the live `state.sqlite`. None is planned. The mirror uses `mirror.sqlite`, and any future `user_version` bump there also needs approval.
12. **.deb install** on the real machine, including how it coexists with or replaces the per-user install and whether to keep the `axentra-onedrive.service.d/` drop-ins.
13. **Cutover.** Stopping OneDrive sync, unlinking, changing the mount, or retiring OneDrive. This is never automatic.
14. **Publishing.** Any git commit, push, PR or release to GitHub, and any change to the Microsoft Entra app registration.
15. **Any unexpected finding.** For example: CLI JSON lacks a SHA-1, a CLI command turns out destructive, an `IntegrityError` occurs, Proton rate-limits or blocks the account, or the crypto migration lands. The mirror pauses and work stops until the user decides.