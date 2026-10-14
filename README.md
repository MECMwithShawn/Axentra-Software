# Axentra Software

Desktop software and browser extensions by Axentra. Each product has its own folder with its documentation,
screenshots, release notes and security scan results. Installers and extension packages are attached to this repository's
[Releases](https://github.com/MECMwithShawn/Axentra-Software/releases), not stored in the folders.

| Product | What it is | Latest | Platforms |
|---|---|---|---|
| [Axentra PDF Suite](PDF-Suite/) | View, edit, convert, compress, sign and protect PDFs, with no accounts or subscriptions | 0.3.4 | Windows 10/11 (64-bit) |
| [Axentra Edge Extension Suite](Edge-Extension-Suite/) | Browser extensions for Microsoft Edge. First: [Axentra Horizon](Edge-Extension-Suite/Horizon/), a calm new tab with Bing and Spotlight backgrounds, your favorites bar on every site, todos, habits, birthday reminders, a daily Bible verse, floating windows, seven page layouts, a tab stash and OneDrive/Google Drive backups | Horizon 0.4.2 | Microsoft Edge, Google Chrome |
| [Axentra Spaces](Spaces/) | Borderless, always-on-top browser windows anywhere on your desktop (YouTube, Spotify, Outlook…) with saved layouts; pairs with Horizon | 0.1.0 | Windows 10/11 (64-bit) |
| [Axentra Vault](Vault/) | A password manager for your own KeePass file, unlocked with your master password plus an iPhone/Android passkey or a YubiKey. No account or cloud | Not released yet (in security review) | Windows 10/11 (64-bit) |
| [Axentra CrystalWire](CrystalWire/) | See what every app sends and receives, with per-app firewall rules and Pi-hole-aware ad blocking planned. No accounts or telemetry | 0.2.2 (in development) | Windows 10/11 (64-bit) |

## Downloading safely

- Download only from this repository's [Releases](https://github.com/MECMwithShawn/Axentra-Software/releases) page.
- Check the file's SHA-256 checksum against the one listed in the product's `releases/` notes:
  `Get-FileHash .\AxentraPDF-0.3.4-Setup.exe -Algorithm SHA256` (PowerShell).
- Each product's `security/` folder links to its VirusTotal scan results.
- Browser extensions: see the [Edge Extension Suite](Edge-Extension-Suite/#installing-an-extension-from-a-release) for installing from a release.

## Repository layout

```
<Product>/
  README.md        overview, features, system requirements, installation
  screenshots/     current screenshots
  releases/        release notes and checksums for each version
  security/        antivirus (VirusTotal) results and code-signing status
```
