# Axentra Software

Desktop software and browser extensions by Axentra. Each product has its own folder with its documentation,
screenshots, release notes and security scan results. Installers and extension packages are attached to this repository's
[Releases](https://github.com/MECMwithShawn/Axentra-Software/releases), not stored in the folders.

| App | Source snapshot version | Published release | Zorin OS |
|---|---|---|---|
| [Command](Command/) | 0.1.11 | 0.1.9 (Windows) | [Installed preview](Command/LINUX.md) |
| [CrystalWire](CrystalWire/) | 0.8.11 | None | [Installed preview](CrystalWire/LINUX.md) |
| [Ghost](Ghost/) | 0.1.4 Windows / 0.1.5 Linux preview | 0.1.4 (Windows) | [Installed preview](Ghost/LINUX.md) |
| [Mail](Mail/) | 0.1.20 | None | [Installed preview](Mail/LINUX.md) |
| [Spaces](Spaces/) | 0.1.2 | None | [Installed preview](Spaces/LINUX.md) |
| [Vault](Vault/) | 0.1.0 | None | [Installed preview](Vault/LINUX.md) |
| [PDF](PDF-Suite/) | 0.3.8 | 0.3.4 (Windows) | [Installed preview](PDF-Suite/LINUX.md) |
| [Archive](Archive/) | 0.1.4 | None | [Installed preview](Archive/LINUX.md) |
| [Horizon](Edge-Extension-Suite/Horizon/) | 0.6.1 | None | [Installed preview](Edge-Extension-Suite/Horizon/LINUX.md) |

Local builds are not automatically public releases. See [current suite status](docs/CURRENT-STATUS.md) and [Linux / Zorin support](docs/LINUX.md).

## Icon source and design kit

The [Axentra Glass + Precision icon kit](Icon-Kit/) contains eight app icons, including
the current outlined Horizon and CrystalWire artwork, their Rust/SVG sources,
ready-to-use PNG/ICO assets, checksum verification,
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
