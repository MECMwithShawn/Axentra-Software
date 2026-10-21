# Axentra suite status — 2026-10-18

All nine applications have native Linux development previews installed and launch-tested on Zorin OS 18.1 / Ubuntu 24.04 x86_64. The source snapshots and Linux changes are maintained in separate private repositories. These are previews with the limits below, not a new stable binary release.


| App | Source snapshot version | Published release | Zorin OS |
|---|---|---|---|
| [Command](../Command/) | 0.1.11 | 0.1.9 (Windows) | [Installed preview](../Command/LINUX.md) |
| [CrystalWire](../CrystalWire/) | 0.8.11 | None | [Installed preview](../CrystalWire/LINUX.md) |
| [Ghost](../Ghost/) | 0.1.4 Windows / 0.1.5 Linux preview | 0.1.4 (Windows) | [Installed preview](../Ghost/LINUX.md) |
| [Mail](../Mail/) | 0.1.20 | None | [Installed preview](../Mail/LINUX.md) |
| [Outlook](../Outlook/) | 0.1.3 | 0.1.3 pre-release (Linux .deb) | [Pre-release package](../Outlook/LINUX.md) |
| [Spaces](../Spaces/) | 0.1.2 | None | [Installed preview](../Spaces/LINUX.md) |
| [Vault](../Vault/) | 0.1.0 | None | [Installed preview](../Vault/LINUX.md) |
| [PDF](../PDF-Suite/) | 0.3.8 | 0.3.4 (Windows) | [Installed preview](../PDF-Suite/LINUX.md) |
| [Archive](../Archive/) | 0.1.4 | None | [Installed preview](../Archive/LINUX.md) |
| [Horizon](../Edge-Extension-Suite/Horizon/) | 0.6.1 | None | [Installed preview](../Edge-Extension-Suite/Horizon/LINUX.md) |

## Axentra OneDrive (added 2026-10-19)

[Axentra OneDrive](../OneDrive/) 0.1.0 is a Linux-only development preview that mirrors the Windows OneDrive client: Files On-Demand, tray and activity panel, file-manager badges and Windows conflict rules. It is installed on Zorin OS 18.1 under `~/.local/share/axentra/apps/onedrive` and syncing a real personal account. Its source is included in this repository under the MIT license. 172 Rust tests passed. Its ice cloud icon is in the Icon Kit (revision 1.6). No binary release is published. Its installer adds a Command manifest, so Command lists it on Linux. See its [Linux notes](../OneDrive/LINUX.md).

## Axentra Outlook (added 2026-10-19)

[Axentra Outlook](../Outlook/) 0.1.3 is a Linux-native mail, calendar, people and to-do client in the new Outlook layout. Its source is published in [Outlook/source](../Outlook/source/), and its `.deb` (installing to `/opt/axentra/outlook` with a Command manifest) is a public GitHub pre-release, `outlook-v0.1.3` (earlier pre-releases remain available). 0.1.1 added encrypted monthly backups to the OneDrive folder; 0.1.2 made the blue header the window title bar; 0.1.3 makes Settings and the other larger dialogs resizable with remembered sizes, and lets you move the compose and read pop-out windows. 433 Rust tests passed; Microsoft Graph sign-in and sync were checked with a real Outlook.com account on a test build. Limits are in its [Linux page](../Outlook/LINUX.md).

## Installation locations

Windows apps use separate folders under `C:\Program Files\Axentra`. Linux previews use separate folders under `~/.local/share/axentra/apps`, with launchers in the application menu. System-wide Linux packaging may use `/opt/axentra/<app>`. User data stays outside the application folders. Command includes Archive (app ID `zip`) and detects all nine installed previews.

## What was verified

- Archive, Command and Mail: native WebKitGTK windows, custom-protocol API calls and POST handling. The GTK-container embedding fix resolved blank window rendering.
- Vault: 235 Rust tests passed; native egui startup. Physical USB passkey testing remains outstanding.
- Mail: 109 ordinary Rust tests and an isolated Secret Service keyring integration test passed. Real account authentication and sending remain untested.
- PDF: 193 tests passed, packaged self-test passed and native window opened.
- Spaces: layout tests and native QtWebEngine page/save check passed.
- Ghost 0.1.5 Linux preview: 17 engine/API tests passed. Live preference round trips passed for 18 GNOME/GTK settings and eight native XFCE settings; every original setting was restored. The active session was XFCE. Other-desktop controls are excluded from active protection counts; configured-endpoint connectivity changes and physical behavior remain unverified.
- CrystalWire: 6 collector tests, including real loopback TCP accounting, passed. Its Linux capture scope remains sampled TCP.
- Horizon: loaded in Brave; both native hosts responded; all nine installed apps appeared in the Command bridge.
- Windows: `cargo check --tests --locked` passed for Archive, Command, Mail and Vault after the portability changes. This is a compilation check, not a new Windows installer release.

## Release status

The published releases listed above remain unchanged. No new public Linux .deb/AppImage or Windows installer was published by this port. Development version numbers describe source snapshots and local installations; they do not imply a matching public download or full Windows feature parity. Detailed remaining work is in [Linux support](LINUX.md) and each application's Linux page.

## Screenshots

Each app has a dated Linux screenshot and capture context. Mail uses demo messages, CrystalWire uses synthetic counters, Spaces uses a local fixture, and the other captures use empty or disposable profiles. Older screenshots remain historical Windows examples.
