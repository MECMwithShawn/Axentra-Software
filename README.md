# Axentra Software

Desktop software and browser extensions by Axentra. Each product has its own folder with its documentation,
screenshots, release notes and security scan results. Installers and extension packages are attached to this repository's
[Releases](https://github.com/MECMwithShawn/Axentra-Software/releases), not stored in the folders.

| App | Local build / development state | Public release | Install subfolder |
|---|---|---|---|
| [Command](Command/) | 0.1.9 local rebuild | 0.1.9 | `Command` |
| [CrystalWire](CrystalWire/) | 0.8.9 | None | `CrystalWire` |
| [Ghost](Ghost/) | 0.1.4 local rebuild | 0.1.4 | `Ghost` |
| [Mail](Mail/) | 0.1.18 installed; 0.1.19 in progress | None | `Mail` |
| [Spaces](Spaces/) | 0.1.2 | None | `Spaces` |
| [Vault](Vault/) | 0.1.0 | None | `Vault` |
| [PDF](PDF-Suite/) | 0.3.8 | 0.3.4 | `PDF` |
| [Archive](Archive/) | 0.1.4 | None | `Archive` |
| [Horizon](Edge-Extension-Suite/Horizon/) | 0.6.0 manifest; newer local work | None | `Horizon` |

Local builds are not automatically public releases. See [current suite status](docs/CURRENT-STATUS.md).

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
