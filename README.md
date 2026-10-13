# Axentra Software

Desktop software and browser extensions by Axentra. Each product has its own folder with its documentation,
screenshots, release notes and security scan results. Installers and extension packages are attached to this repository's
[Releases](https://github.com/MECMwithShawn/Axentra-Software/releases), not stored in the folders.

| Product | What it is | Latest | Platforms |
|---|---|---|---|
| [Axentra PDF Suite](PDF-Suite/) | View, edit, convert, compress, sign and protect PDFs, with no accounts or subscriptions | 0.3.1 | Windows 10/11 (64-bit) |
| [Axentra Horizon](Edge-Extension-Suite/Horizon/) | A calm new tab with Bing and Spotlight backgrounds, focus, todos, habits, birthday reminders and a tab stash. Part of the [Edge Extension Suite](Edge-Extension-Suite/) | 0.3.0 | Microsoft Edge, Google Chrome |

## Downloading safely

- Download only from this repository's [Releases](https://github.com/MECMwithShawn/Axentra-Software/releases) page.
- Check the file's SHA-256 checksum against the one listed in the product's `releases/` notes:
  `Get-FileHash .\AxentraPDF-0.3.1-Setup.exe -Algorithm SHA256` (PowerShell).
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
