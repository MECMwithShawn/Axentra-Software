# Axentra Software

Desktop software and browser extensions by Axentra. Each product has its own folder with its documentation,
screenshots, release notes and security scan results. Installers and extension packages are attached to this repository's
[Releases](https://github.com/MECMwithShawn/Axentra-Software/releases), not stored in the folders.

| Product | What it is | Latest | Platforms |
|---|---|---|---|
| [Axentra Command](Command/) | One place for every Axentra app: see what's installed and running, keep the suite in one tray icon, choose what starts at sign-in, and install the other apps from these releases with SHA-256 checks. Start here | 0.1.9 (published; local rebuild prepared) | Windows 10/11 (64-bit) |
| [Axentra PDF Suite](PDF-Suite/) | View, edit, convert, compress, sign and protect PDFs, with no accounts or subscriptions | 0.3.8 (prepared; unpublished) | Windows 10/11 (64-bit) |
| [Axentra Edge Extension Suite](Edge-Extension-Suite/) | Browser extensions for Microsoft Edge. First: [Axentra Horizon](Edge-Extension-Suite/Horizon/), a calm new tab with Bing and Spotlight backgrounds, your favorites bar on every site, todos, habits, birthday reminders, a daily Bible verse, floating windows, seven page layouts, a tab stash and OneDrive/Google Drive backups | Horizon 0.4.2 | Microsoft Edge, Google Chrome |
| [Axentra Spaces](Spaces/) | Borderless, always-on-top browser windows anywhere on your desktop (YouTube, Spotify, Outlook…) with saved layouts; pairs with Horizon | 0.1.0 | Windows 10/11 (64-bit) |
| [Axentra Vault](Vault/) | A password manager for your own KeePass file, unlocked with your master password plus an iPhone/Android passkey or a YubiKey. No account or cloud | 0.1.0 (prepared; unpublished) | Windows 10/11 (64-bit) |
| [Axentra CrystalWire](CrystalWire/) | See what every app sends and receives, with per-app firewall rules and Pi-hole-aware ad blocking planned. No accounts or telemetry | 0.2.2 (in development) | Windows 10/11 (64-bit) |
| [Axentra Ghost](Ghost/) | Stop Windows telemetry, ads and tracking and remove preinstalled apps, with exact undo, drift watch that re-applies what updates turn back on, and a live telemetry monitor | 0.1.4 (published; local rebuild prepared) | Windows 10/11 (64-bit) |
| [Axentra Mail](Mail/) | An Outlook.com mail client that filters spam and phishing and files promotions with AI that runs on your own PC. Your mail is never sent to an AI company | Not released yet (in testing) | Windows 10/11 (64-bit) |

## Icon source and design kit

The [Axentra Glass + Precision icon kit](Icon-Kit/) contains the current seven app icons,
their complete Rust renderer, ready-to-use PNG/ICO assets, checksum verification,
integration instructions and a guide for creating matching icons for new apps.
The glass PDF **document** icon is preserved separately from the PDF application icon.

![Axentra app icon family](Icon-Kit/references/production-preview.png)

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
