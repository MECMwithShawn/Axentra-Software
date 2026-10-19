# Axentra icon kit — Glass + Precision, v1.4

This is the canonical, self-contained source kit for the Axentra app icons adopted on
October 14, 2026, with subsequent additions. It includes all ten production icons:
Command, Ghost, Vault, CrystalWire, Horizon, PDF, Spaces, Mail, Archive and Outlook.
Use this kit to reproduce the existing icons or
give a new app the same glass tile, geometry, lighting and optical-size treatment.

Revision 1.4 adds [Archive](ARCHIVE.md) using its original installed lime archive icon.
It includes the preserved Windows ICO, eight PNG sizes, an extraction/export script,
and Archive in the production contact sheet and orbital preview. All 72 exports for
the existing eight apps remain byte-identical.

Revision 1.3 updates Horizon to its outlined gold sunrise and CrystalWire to its
lilac outlined crystal. Their editable [SVG masters and export rules](OUTLINE-ICONS.md)
now live here alongside the exact PNGs and Windows ICOs used by the applications and
website. The other six app icons and the PDF document icons remain byte-identical.

Revision 1.2 added [Axentra Mail](MAIL.md): a rose-pink envelope with a recessed flap,
using the existing glass tile and optical-size treatment. That revision kept all seven
earlier app icons byte-identical. Ready-to-use files: [Windows ICO](assets/mail.ico) and
[512 px PNG](assets/mail-512.png).

Revision 1.1 brought Vault closer to its original circuit-halo identity: a smaller icy-blue
lock, thin cyan rings and restrained glow inside the existing suite tile. See the
[Vault design notes](VAULT.md) and [revision history](CHANGELOG.md). That revision preserved the other six app
icons byte for byte.

![Production icons, including small-size samples](references/production-preview.png)

Six editable masters use **procedural Rust geometry**. Horizon and CrystalWire use
the approved **SVG masters** in `svg/`, rendered with pinned Sharp 0.35.5. The complete
export runs both renderers, then extracts Archive from its preserved ICO. Archive's
original procedural/vector source was unavailable; the ICO is its raster master.
Running Cargo alone omits Archive and produces the historical filled versions of the
two outline icons. No image-generation service or sibling app repository is needed.

## Use the existing icons

Ready-to-use files are in [`assets/`](assets/):

- PNGs: 16, 24, 32, 48, 64, 128, 256 and 512 px per app. Archive's 128/512 px
  exports are resized from its native 256 px frame; see [provenance](ARCHIVE.md).
- Windows ICOs: Horizon/CrystalWire have seven frames at 16, 24, 32, 48, 64, 128 and
  256 px; the other seven have eight at 16, 20, 24, 32, 40, 48, 64 and 256 px.
- `preview.png`: all ten apps in catalog order, at 160 px and magnified 32/16 px.

**PDF documents keep the separate [glass PDF file icon](document-icons/README.md).**
`assets/pdf.ico` identifies the application; it must not replace the document icon.

## Reproduce the current set

From this `Icon-Kit` directory, with Rust/Cargo, Python 3 and Node.js/npm available:

```powershell
python scripts/verify.py
npm install --prefix svg --ignore-scripts
python scripts/export.py out
python scripts/verify.py --rendered out
```

The first check verifies the committed source and assets against `manifest.json`.
The npm command installs the SVG renderer dependency. `export.py` creates a fresh
complete export. The final check requires every regenerated
PNG and ICO, including the contact sheet, to match the approved files **byte for byte**.
Nothing writes to an application, installer, shortcut or registry during generation.

The verified baseline used Rust 1.99.0 and Node.js 24.19.0 on Windows x64; full compiler
and SVG library details are recorded in [`build-info.json`](build-info.json). The Rust
crate has no external dependencies; the SVG and Archive extraction passes use Sharp.
Rust floating-point transcendental functions can differ across targets/toolchains:
always run the checksum comparison before calling another environment byte-identical.
Use the checked-in assets when exact bytes are needed without regenerating them.

## Animate the icon family

The [motion component](motion/README.md) adds orbital movement, spring hover bounce,
curved 3D spins and mirrored reverse faces using all nine icons. It is an
original, dependency-free WebGL implementation. See `motion/index.html` for the demo
or build a portable preview with `python motion/build_preview.py`. Keyboard, reduced
motion, narrow screens and static fallback are covered. Source artwork and Windows
ICOs remain unchanged.

## Create a matching icon for another app

1. Read the [design specification](DESIGN.md).
2. Follow [Adding an app](ADDING-AN-APP.md), including the compilable example.
3. Export all icons and confirm existing apps did not change.
4. Follow the [integration and release guide](INTEGRATION.md).

For another developer or coding assistant, [the handoff brief](HANDOFF.md) provides a
ready-to-use starting request. Start with this kit; the older generic prompt at the
repository root is historical and does not describe the current production renderer.

## Contents

| Path | Purpose |
|---|---|
| `renderer/src/suite.rs` | Six current procedural icons; historical filled Horizon/CrystalWire geometry |
| `svg/`, `OUTLINE-ICONS.md` | Current Horizon/CrystalWire masters, optical-size rules and SVG exporter |
| `raster/`, `ARCHIVE.md` | Preserved Archive ICO master, exact frame extraction and derived-size provenance |
| `renderer/src/main.rs` | Deterministic PNG/ICO encoder and contact-sheet exporter |
| `renderer/Cargo.toml`, `Cargo.lock` | Standalone Rust crate, no dependencies |
| `assets/` | Approved production PNG/ICO exports and contact sheet |
| `motion/` | Orbital animation component, demo, portable-preview builder and browser checks |
| `MAIL.md` | Mail symbol specification and build handoff |
| `OUTLOOK.md` | Outlook symbol specification (calendar page and envelope) |
| `manifest.json`, `build-info.json` | SHA-256 baseline and build provenance |
| `document-icons/` | Preserved PDF document ICOs and their legacy SVG design sources |
| `references/` | Original concept, current production preview, Vault reference, PDF installer preview |
| `scripts/verify.py` | Standard-library integrity and regeneration checks |
| `scripts/export.py` | Complete Rust + SVG + Archive export and nine-app contact sheet |
| `scripts/sync_assets.py` | Portable, dry-run-by-default copier for the current app projects |
| `scripts/verify_binary.py` | Compare a Windows executable's embedded icon against an ICO |

## Source ownership

Make future icon changes here first. Command and Ghost embed copies of `suite.rs` in
their builds; Command uses the outline SVGs for Horizon and CrystalWire. The other apps
consume exported assets. The sync script updates these sources and exports explicitly.
Application repositories can contain unreleased work, so review the
copy plan and coordinate rebuilds before installing. The public kit contains no local
machine paths, installers, application data or development logs.

AI-generated concept artwork in `references/approved-concept.png` informed the design.
It records the initial family direction, before Vault's circuit-halo revision; it is a
visual reference only. The actual shipping designs are defined by the renderer
and `assets/`, so exact reproduction never depends on regenerating that concept.
