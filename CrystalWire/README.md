<img src="logo.png" alt="Axentra CrystalWire logo" width="96" align="right">

# Axentra CrystalWire

<!-- local-release-preparation -->
## Prepared version: 0.8.2

This version is prepared locally and is awaiting approval to upload a GitHub Release draft.
The previously documented version was **0.2.2**.
See [release notes, build status and SHA-256 hashes](releases/0.8.2.md) and [changelog](CHANGELOG.md).

- Advanced the distribution documentation from 0.2.2 to the real 0.8.2 workspace version, with the current Rust/WebView2 architecture and refreshed functional spec.
- Security Center with reputation checks and review queue; live traffic graph, app identity and connection-owner attribution; Home/Work/Public profiles and consent-gated discovery/network map.
- Read-only ASUS router access and network/DNS configuration-health evidence, with CLI/MCP tools and AI-agent task handoff.
- Opt-in Ghost/Pi-hole domain synchronization with home-network validation, bounded authenticated command-pipe handling and queued removal from earlier Pi-holes.
- Upgrade/Fresh installation modes, service-start recovery and Setup console fixes; shared Command tray and background launch.
- Per-app WFP firewall rules and a local DNS blocking engine remain planned, rather than being advertised as implemented.

Prepared `AxentraCrystalWire-0.8.2-Setup.exe` is a per-machine installer and requires administrator rights for the recording service. Upgrade preserves history/settings; Fresh install is an explicit destructive choice, with removing saved logins separate. This task did not install, stop or reconfigure the running service.

Pending: The build includes existing uncommitted source icon assets; their hashes are preserved, but a source commit alone does not reproduce this working tree.; Code signing and fresh VirusTotal scan pending; installer not installed during this task.; Draft release assets remain local at Shawn's request. Pushing and publishing still require approval.
<!-- /local-release-preparation -->

A Windows network monitor with per-app traffic history, a live graph, DNS and Pi-hole attribution,
a Security Center, network maps and configuration-health checks. It records Windows network events
without reading packet contents. There is no Axentra account, subscription or telemetry.

## Current features

| Area | Available in 0.8.2 |
|---|---|
| Traffic and attribution | Live stacked traffic graph, per-app and per-server history, country/network-owner database, service-aware app names and icons; connection-owner attribution fixes prevent busy unrelated processes being credited with traffic |
| DNS and Pi-hole | Per-app DNS history, which DNS server answered, Pi-hole block attribution and warnings when DNS bypasses Pi-hole |
| Security Center | File signature and Defender checks, review queue, changed-file alerts, optional online hash reputation checks, tray review/threat badge |
| Home, Work and Public | Home device discovery requires consent. Work/Public views avoid discovery and can report LAN contacts; unknown networks default to Public |
| Network map | Device links, details, labels and SVG export; optional ASUS router connection adds wired/Wi-Fi and guest-network information |
| Configuration health | Read-only checks of this PC, connected router and Pi-hole; evidence explains what was reported and where a setting can be fixed |
| AI-agent handoff | A finding can be handed to an agent with evidence and ground rules; `cw mcp` exposes read-only findings and recheck tools |
| Ghost integration | With explicit setup, the service synchronizes Ghost's selected domains to the configured home Pi-hole and removes only Ghost-tagged entries; home-network checks and queued removals protect switching networks |
| Command integration | Shared suite tray, review counts and actions, background startup; its own tray returns when Command stops hosting |
| Upgrade handling | Upgrade preserves history/settings; Fresh install is an explicit choice. Setup shows intervening changes and verifies that the recording service starts |

Per-app allow/block firewall rules, ask-to-connect and a local DNS blocking engine are future milestones.
The current app does not install those WFP firewall rules. Ghost's opt-in Pi-hole synchronization can
change Ghost-tagged deny-list entries on your Pi-hole.

## Installation and requirements

- Windows 10 (1809 or later) or Windows 11, 64-bit; Microsoft Edge WebView2 Runtime.
- Administrator rights for Setup, because the recording service runs as SYSTEM.
- Prepared installer: `AxentraCrystalWire-0.8.2-Setup.exe`. Check its SHA-256 in the release notes before running it.
- Setup installs the window, recording service and `cw` command-line tool. Choose Upgrade to retain data;
  Fresh install clears history/settings, and removing saved logins is a separate choice.
- This preparation task builds the installer only; it does not install or upgrade the running service.

The installer is unsigned. SmartScreen may warn and Smart App Control can block it. Signing and a
VirusTotal scan of this exact build are pending.

## Privacy and optional connections

Traffic history is stored locally in SQLite. Network capture uses Windows ETW events, without a
packet-capture driver, HTTPS interception or a root certificate. Optional network features include
reverse DNS, the public owner database, connections to your Pi-hole/router, and opt-in file-hash
reputation checks. Online reputation requests send hashes rather than uploading files.

Router access is read-only, restricted to the trusted Home setup, with its certificate pinned.
Configuration-health checks report findings; they do not automatically change router/DNS settings.
Ghost synchronization writes only the opted-in Ghost-tagged Pi-hole entries.

## Technical reference

See [SPEC.md](SPEC.md) for the implemented monitor milestone and the remaining firewall/blocking plans.
The service, command-line tools and window are written in Rust; the window uses Microsoft Edge WebView2.
The source workspace declares the MIT license.
