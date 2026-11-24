# Axentra suite status — 2026-10-21

All ten catalog entries have native Rust Linux builds and local Debian packages. Nine apps and the separately named **PDF Rust preview** are installed on this Zorin 18.1 / Ubuntu 24.04 x86_64 machine. **The complete migration is not finished:** PDF still lacks major original workflows, and live account, hardware and Wayland checks remain outstanding.


| App | Source snapshot version | Published release | Zorin OS |
|---|---|---|---|
| [Command](../Command/) | 0.1.11 | 0.1.9 (Windows) | [Installed preview](../Command/LINUX.md) |
| [CrystalWire](../CrystalWire/) | 0.8.11 | None | [Installed preview](../CrystalWire/LINUX.md) |
| [Ghost](../Ghost/) | 0.1.4 Windows / 0.1.5 Linux preview | 0.1.4 (Windows) | [Installed preview](../Ghost/LINUX.md) |
| [Mail](../Mail/) | 0.1.20 | None | Retired; replaced by Axentra Mail 0.1.9 ([Outlook](../Outlook/)) |
| [Outlook](../Outlook/) (now Axentra Mail) | 0.1.9 | 0.1.9 pre-release (Linux .deb) | [Pre-release package](../Outlook/LINUX.md) |
| [Spaces](../Spaces/) | 0.1.2 | None | [Installed preview](../Spaces/LINUX.md) |
| [Vault](../Vault/) | 0.1.0 | None | [Installed preview](../Vault/LINUX.md) |
| [PDF](../PDF-Suite/) | 0.3.8 | 0.3.4 (Windows) | [Installed preview](../PDF-Suite/LINUX.md) |
| [Archive](../Archive/) | 0.1.4 | None | [Installed preview](../Archive/LINUX.md) |
| [USB](../USB/) | 0.1.1 | None | [Installed preview](../USB/LINUX.md) |
| [Horizon](../Edge-Extension-Suite/Horizon/) | 0.6.20 | 0.6.20 pre-release (Linux .deb) | [Pre-release package](../Edge-Extension-Suite/Horizon/LINUX.md) |
| [Notepad](../Notepad/) | 0.1.5 | None | [Installed preview](../Notepad/LINUX.md) |

## Axentra OneDrive (added 2026-10-19)

[Axentra OneDrive](../OneDrive/) 0.1.0 is a Linux-only development preview that mirrors the Windows OneDrive client: Files On-Demand, tray and activity panel, file-manager badges and Windows conflict rules. It is installed on Zorin OS 18.1 under `~/.local/share/axentra/apps/onedrive` and syncing a real personal account. Its source is included in this repository under the MIT license. 172 Rust tests passed. Its ice cloud icon is in the Icon Kit (revision 1.6). No binary release is published. Its installer adds a Command manifest, so Command lists it on Linux. See its [Linux notes](../OneDrive/LINUX.md).

## Axentra Outlook (added 2026-10-19)

[Axentra Outlook](../Outlook/) 0.1.9, renamed Axentra Mail and replacing the earlier Mail app, is a Linux-native mail, calendar, people and to-do client in the new Outlook layout. Its source is published in [Outlook/source](../Outlook/source/), and its `.deb` (installing to `/opt/axentra/outlook` with a Command manifest) is a public GitHub pre-release, `outlook-v0.1.9` (earlier pre-releases remain available). 0.1.1 added encrypted monthly backups to the OneDrive folder; 0.1.2 made the blue header the window title bar; 0.1.3 made Settings and the other larger dialogs resizable with remembered sizes and made the compose and read pop-out windows movable; 0.1.4 shows a Microsoft account's own profile photo instead of initials on the account button, in the account menu and in Settings › Accounts. 452 Rust tests passed; Microsoft Graph sign-in and sync were checked with a real Outlook.com account on a test build, and the account photo with the user's own Outlook.com account on a debug build. Limits are in its [Linux page](../Outlook/LINUX.md).

## Axentra Notepad (added 2026-10-21)

[Axentra Notepad](../Notepad/) 0.1.5 is a Linux text and code editor with Windows Notepad, Notepad++ and VS Code layouts: tabs and sessions, highlighting for 46 languages, multiple cursors, find and replace across folders and 25 encodings. Its source is published in [Notepad/source](../Notepad/source/); there's no published package yet. 0.1.3 made closing reliable (a native save prompt and an exit watchdog) and turned off GPU rendering after WebKit's GPU renderer froze the window on a hybrid Intel/NVIDIA laptop; 0.1.4 keeps highlights aligned when the font loads late; 0.1.5 stops lines overlapping after a font size change, adds Ctrl+scroll wheel zoom and gives each layout its usual font size and spacing. 21 Rust tests and 715 editor and highlighter tests passed. Limits are in its [Linux page](../Notepad/LINUX.md).

Command's local catalog and GNOME Software identify all ten package files with their expected names and original artwork. PDF is clearly labeled **Axentra PDF (Rust preview)**. Local package paths and SHA-256 checksums are registered by `tools/register-local-packages.py`; no public GitHub release was published.

PDF now implements transactional document editing, forms, page organization, metadata, attachments, printing, password security, bookmarks, layers, clipboard/image/Office import, OCR, text/HTML/image/Office/PDF-A export and inline text editing. Preview 12 is installed and available through Command. All seven independent check scripts (second reader/structure, images, links, Office, PDF/A, text editing and exports) were rerun on the installed preview 12 outputs. Word/Excel/PowerPoint files pass independent readers and LibreOffice round trips; PDF/A-1b, 2b and 3b fixtures pass veraPDF validation. Text edits preserve pending redaction dictionaries, unrelated pixels, images, mixed font/colour runs and supported embedded Unicode fonts. Copy/Cut restrictions, cancellation and undo/redo pass. Image placement, replacement, extraction and deletion retain the original controls; twelve native checks and independent pixel comparisons pass. Shared images and nested forms are handled, and Word exports convert CMYK images correctly. Preview 12 adds link editing and integrated window controls. Evidence: `outputs/pdf-linux/verification/preview12-summary.json` and `installed-preview12/` (preview 11: `preview11-summary.json` and `installed-preview11/`).

Digital signing/certificate validation, full text/Office layout fidelity, Unicode font selection/shaping, OCR language downloads, accessibility, recovery and full original interface parity remain work. Signed documents open read-only. Directly rotated/skewed source text and invisible OCR text over scans are explicitly refused pending migration. Native checks use X11/Xvfb; they do not certify Wayland or a clean machine.


## Window-controls update installed — 2026-10-21

All seven window-control packages are installed: CrystalWire 0.8.11-3, Ghost 0.1.4-3, Mail 0.1.21-3, Archive 0.1.4-3, Vault 0.1.0-4, USB 0.1.1-1 and PDF 0.3.8~rust-preview.12. The owner ran `outputs/window-chrome/install-updates.sh` from a normal Terminal with ordinary administrator authentication. It rechecked all seven SHA-256 values, and APT recorded a single transaction (07:50:14–07:50:16 CDT, 12:50 UTC) upgrading exactly these packages. An earlier attempt from an agent session failed with `pkexec must be setuid root` because that process had `NoNewPrivs` set; it was not a declined password prompt. Command, Spaces and Horizon were not changed.

`work/smoke/verify-window-chrome-install.py` confirms that every packaged file and symlink matches the installed payload (94 regular files). `work/smoke/installed-desktop-identity.py` records that each desktop entry validates, its launcher resolves to an executable owned by the package, no user-local entry overrides it, and its icon is a packaged hicolor file. CrystalWire's service restarted on upgrade and is active and enabled. A read-only `cw status` run after the upgrade shows it writing live history (`work/logs/crystalwire-installed-0.8.11-3-status.log`). Ghost's drift timer remains disabled because it is opt-in.

The main windows use controls inside their existing interface, without an extra system title bar. Sixty-seven native checks cover minimize, maximize/restore, dragging, resizing from the bottom-right corner, double-click, minimum sizes, close-to-tray and single-instance reopening. They combine real xdotool mouse input with window-property probes. All eight resize handles are implemented, but only the corner is exercised. The same 67 checks were rerun against the installed executables and pass. Vault's Classic and Floating unlock layouts are both checked, and its existing editor/unsaved-data UI tests pass. PDF's existing Save/Discard/Cancel close decisions also pass. Command and Spaces already had integrated controls; Horizon uses its browser's controls. Native file pickers and auxiliary dialogs retain their toolkit decorations. Windows and Wayland interactions remain unverified.

Preview 12 also adds the original PDF Add/Edit Links workflow: transactional creation, editing and deletion; internal/named page targets; confirmed web links; shared-annotation isolation; and rotated/cropped rectangle geometry. Fifty-eight model tests pass, as do native document workflows and independent structural/second-reader checks on 43 PDFs. Six link PDFs also pass content/pixel preservation checks. The installed `/opt` binary passes the native workflows with host libraries, and its 43 PDFs plus image, link, Office, PDF/A (veraPDF), text-edit and export outputs pass independent checks. Exact destination zoom/Fit behaviour and accepted external-browser handoff remain unverified. The broader PDF migration is still unfinished.

After installation, the Command runtime started by the desktop launcher (`~/.local/share/axentra/command/runtime/AxentraCommand`) discovers all ten local installers with the correct names, icons and checksums. It no longer offers any of them as an update. GNOME Software reports each package as installed. Horizon's catalog entry is now the installed 0.6.3-2 (see the Horizon sections below).

Current evidence: `outputs/window-chrome/status.json`, `installed-verification.json`, `installed-desktop-identity.json` and the `installed-<app>/` folders beside it (`installed-attempt1/` is the failed first attempt, explained in its README), `outputs/pdf-linux/verification/preview12-summary.json` and `installed-preview12/`, `outputs/command-linux/verification/suite-install-catalog.json` and `work/logs/suite-chrome-catalog-installed.log`.

## Horizon 0.6.3 reconciliation — 2026-10-21

Horizon 0.6.3-1 was built and installed on 2026-10-20 by a separate task (`Codex/2026-10-20/can-x20`), with `dpkg` rather than APT. It has now been reconciled with this task's sources. It was not downgraded.

- **Source.** 0.6.3 is exactly this task's sealed 0.6.2 source plus that task's `integration.patch`; two files also changed from CRLF to LF line endings. The patch is kept in `outputs/horizon-linux/reconcile-0.6.3/patch-0.6.3/`, and the 0.6.2 originals in `source-0.6.2/`. This task's Horizon (`stillpoint`) and Command `horizon_mail.rs`/`horizon.rs` sources now contain those 0.6.3 files byte-for-byte.
- **Changes.** The mail card adds Outlook items, source labels and an **Open Outlook** button. The bridge reads the local Outlook cache read-only and never returns message bodies. Outlook items open the Outlook app, not a specific message. Horizon now bundles its own Command bridge (`HorizonCommand`) instead of calling the installed Command app, so the package gains GTK/WebKit dependencies.
- **Installed build.** The installed files match the held 0.6.3-1 package (120 files), and `dpkg --verify` is clean. The packaged extension matches the source (105 files; the manifest differs only by the packaging key). `HorizonResources` is byte-identical to 0.6.2-1.
- **Tests.** 28 Command tests (including the new Outlook privacy test) and 9 Rust companion tests pass. Against the installed build, these also pass:
  - six native-messaging checks with fictional Mail/Outlook caches;
  - mail-card rendering;
  - system-CRX installation, storage persistence and the new-tab override in a disposable Brave profile;
  - the companion bridge integration, with a disposable data home. A hidden-app marker proves the bridge used it (`installed-companion-0.6.3/isolation.json`), and no Mail or Outlook cache existed there, so no real mail was read.
- **Limitation: provenance.** `HorizonCommand` could not be rebuilt byte-for-byte. Rebuilds with the same compiler, lockfile and source are deterministic but differ by about 3.3 KB of machine code. The original release target directory no longer exists, so the remaining difference could not be identified (`reconcile-0.6.3/horizoncommand-reproduction.json`).
- **Limitation: sync time.** The merged summary reports `synced: 0`, so the card no longer shows Mail's last-sync time.
- **Command app.** Horizon 0.6.3 uses its own bundled bridge, not the installed Command app. Later Command builds from this source also include the bridge change.
- **Not tested.** Accepted Open Mail/Open Outlook launches, real accounts and Wayland.

**Catalog registration was deferred, then completed with 0.6.3-2 (next section).** With 0.6.3-1 registered, GNOME Software identified it correctly as installed. But Command offered the installed build as an update. Command takes an installed extension's version from the browser profile (`0.6.3`) and compares it with the Debian version (`0.6.3-1`) using dpkg, which ranks `0.6.3-1` higher. This is a latent Command bug in `src/probe.rs`; the older 0.6.2-1 entry had hidden it. The 0.6.3-1 installer and CRX are therefore held in `outputs/horizon-linux/reconcile-0.6.3/package/` with their checksums, and the catalog still lists 0.6.2-1. Command does not offer a Horizon update, so nothing would be downgraded. Registering 0.6.3-1 should wait for a Command fix that compares Debian versions for packaged extensions.

This work also changed test tooling:

- `stillpoint/tools/smoke-package-linux.cjs` now records the extension version Brave loaded. This is test-only and not part of the 0.6.3 patch.
- `work/smoke/suite-catalog.py` gained optional `AXENTRA_COMMAND_BINARY` and `AXENTRA_SOFTWARE_TIMEOUT` settings, because GNOME Software lookups took about 94 seconds.
- Three new harnesses: `work/smoke/horizon-native-host.py`, `horizon-mail-card.mjs` and `horizon-companion-isolated.py`.

Evidence: `outputs/horizon-linux/verification/reconcile-0.6.3.json`, the `installed-*-0.6.3/` folders beside it, and `reconcile-0.6.3/suite-install-catalog-with-0.6.3-registered.json`.

## Command extension-version fix and Horizon 0.6.3-2 — 2026-10-21

The owner approved this work, and it follows `outputs/command-linux/extension-version-fix/INSTALL-PLAN.md`, steps 1–3, 5 and 6.

- **Fix.** `src/probe.rs` now compares a local `.deb` build, or a `.deb` GitHub release, with the installed package version (dpkg) instead of the version the browser reports. The update note names the version it compared, and the status data includes `package_version`. The patch (`probe-extension-version.patch`) and a changelog line are in the shared Command source. All 30 Command tests pass, including a new test of these cases.
- **Behaviour.** On the same binary, the fixed build no longer offers the installed Horizon build. It still offers a newer Debian revision or release, and doesn't offer an older one. Across the 16 apps on this machine, it reports the same status as the previous runtime.
- **Runtime.** The user-local runtime that the menu launcher and sign-in start was replaced with this build at 13:11. The previous one is in `extension-version-fix/runtime-backup/`. The old process kept running after a tray Exit, so it was stopped with a normal termination signal at the owner's request.
- **Horizon 0.6.3-2.** Horizon bundles its own copy of this code (`HorizonCommand`), so it was rebuilt as 0.6.3-2. The owner installed it with APT at 12:53. The package differs from 0.6.3-1 only in its version, `HorizonCommand` (the tested fixed build) and the re-signed CRX, whose 105 extension files are unchanged. `dpkg --verify` is clean and the installed payload matches the package. The installed bridge passes the six fixture checks, and Horizon's new-tab launcher no longer offers the installed build but still offers a newer one. `stillpoint/tools/package-linux.py` gained `--revision` for this.
- **Catalog.** `outputs/horizon-linux/SHA256SUMS.txt` lists 0.6.3-2, and the registrar registered it. The catalog check with the fixed runtime reports it installed and not offered as an update.
- **Not done here.** The system Command package in `/opt` is the other session's 0.1.17-1, installed at 12:50, and doesn't contain the fix (plan step 4). `ui/app.js` and `src/horizon.rs` still compare a GitHub release with the browser-reported version, which can't happen while no Horizon GitHub release exists.

Evidence: `outputs/horizon-linux/verification/horizon-0.6.3-2.json`, `installed-native-0.6.3-2/` and `outputs/command-linux/extension-version-fix/` (`shared-build/`, `summary.json`). `work/smoke/seal-horizon-0.6.3-2.py` supersedes `seal-horizon-0.6.3.py`.

## Source changes from other work — 2026-10-21

Between 08:56 and 09:01 CDT, after the window-control records were sealed, another session ran `work/smoke/sync-premium-suite-icons.py` (backups and logs in `outputs/suite-icons-update/`).

- It rewrote icon assets in the Command, Ghost, Mail, PDF, Spaces, Vault and Archive sources and in USB's public-repository source, including Ghost's `src/suite_icon.rs`.
- Between 09:00 and 09:03 it rebuilt the Command, Ghost, Mail, Archive, Spaces, USB, PDF and Vault release binaries in place. `work/sources/axentra-command/target/release/AxentraCommand` therefore no longer matches the user-local Command runtime, which is unchanged. That rebuild also picked up this work's Horizon 0.6.3 bridge source.

Later that day, other sessions also made these Command changes:

- At 10:24–10:36 they bumped Command to 0.1.17 and changed `src/probe.rs` for icon refresh. They built `axentra-command_0.1.17-1`, replaced the user-local runtime with it, and restarted Command.
- At 12:50 `axentra-command` 0.1.17-1 was installed to `/opt`.
- At 13:08 another Claude session built `axentra-command_0.1.17-2` from the shared tree, which by then contained the extension-version fix. That package isn't installed. It is now the newest file in `outputs/command-linux`, so Command offers it as a Command update.

The installed packages and their sealed evidence are unaffected. The release binaries named in the pre-install window-check results have since been replaced, so the installed-executable reruns (`outputs/window-chrome/installed-*/`) are the current evidence. `work/smoke/seal-window-chrome.py` refuses to re-seal until the icon changes are rebuilt, packaged and tested. The Horizon reconciliation seal re-records only the files it changed. It reports the other session's changes to recorded files as unsealed: two Command suite-icon files, Ghost's `src/suite_icon.rs` and PDF's `axentra-256.png`.
