# Axentra suite status — 2026-10-16

| App | Local build / development state | Public release | Install subfolder |
|---|---|---|---|
| [Command](../Command/) | 0.1.9 local rebuild | 0.1.9 | `Command` |
| [CrystalWire](../CrystalWire/) | 0.8.9 | None | `CrystalWire` |
| [Ghost](../Ghost/) | 0.1.4 local rebuild | 0.1.4 | `Ghost` |
| [Mail](../Mail/) | 0.1.18 installed; 0.1.19 in progress | None | `Mail` |
| [Spaces](../Spaces/) | 0.1.2 | None | `Spaces` |
| [Vault](../Vault/) | 0.1.0 | None | `Vault` |
| [PDF](../PDF-Suite/) | 0.3.8 | 0.3.4 | `PDF` |
| [Archive](../Archive/) | 0.1.4 | None | `Archive` |
| [Horizon](../Edge-Extension-Suite/Horizon/) | 0.6.0 manifest; newer local work | None | `Horizon` |

## Installation layout

All updated Windows desktop apps use a separate folder under `C:\Program Files\Axentra`. Horizon uses this root for its Windows native companion. Old app folders may remain for recovery; do not delete user data during migration. Command includes Archive (`zip`) and Mail, and preserves custom app manifests.

## Build and publication status

Only Command 0.1.9, Ghost 0.1.4 and PDF 0.3.4 were verified as published releases during this refresh. No new binary releases were published. Same-version local rebuilds are not interchangeable with published artifacts or checksums. Mail, CrystalWire and Horizon have ongoing development; the notes below describe completed work separately from pending requests.

### Command

Mail and Archive are built into the nine-app catalog. Archive uses app ID `zip`. The local rebuild differs from published 0.1.9; assign a new version before releasing it.

### CrystalWire

Security Center, Firewall paging and Traffic controls are updated. CrystalWire keeps its own tray icon. Firewall views monitor traffic; Windows Firewall enforces policy. Network map, DNS layout, device identity and Command icon work is still in progress.

### Ghost

Windows privacy controls with exact undo and drift monitoring. The local installer uses the shared suite root; published installer hashes still describe the original release.

### Mail

Hover summaries use cached bodies over 5,000 characters and do not mark messages read. Disregard and Archive preview actions are being added. Local AI uses Ollama; optional reputation and logo services can contact external providers.

### Spaces

Floating WebView2 windows and saved layouts; integrates with Horizon and Command.

### Vault

KeePass password storage with passkey/security-key options, backup retention and restore. Audit fixes and a second-pass checklist remain local; a checklist is not security certification.

### PDF

Local PDF development and packaging are ahead of the public release. Windows MSI uses the shared suite directory and removes the older product early during major upgrades.

### Archive

UI and binary are named Axentra Zip; Command shows Axentra Archive. Create ZIP/7z and extract/test archives. RAR creation requires the user's separately licensed WinRAR; no RAR writer is bundled.

### Horizon

Browser new-tab extension with a Windows native companion. Mergeable Apps/Resources panels and local music improvements are present. Calendar integration, Common Sites merging and driver-warning handling remain active work. The manifest version has not advanced with all local changes.

## Screenshots

Mail includes a 0.1.18 demo-mailbox capture; Archive includes its 0.1.4 empty state; Horizon includes a recorded-telemetry panel-merging preview. Existing screenshots for other apps remain historical UI examples, not verification of the latest local build. Current CrystalWire captures include personal network information and were intentionally not published. Retake a sanitized current capture before the next release.
