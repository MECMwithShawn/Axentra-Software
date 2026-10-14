# Integration and release guide

Generate and verify the source kit before changing app projects. Updating a source ICO
does not change the icon already embedded in an installed executable.

## Synchronize the current app checkouts

The optional copier takes an explicit parent directory; it never assumes a username
or workstation path. From `Icon-Kit`, preview the copy plan:

```powershell
python scripts/verify.py
python scripts/sync_assets.py --code-root D:\code --apps command ghost vault crystalwire horizon pdf spaces
```

The expected sibling layout and file destinations are below. For another layout, edit
the `PROJECTS` map in the script. Missing project folders cause an error before writes.
Review each project's changes before applying; icon files and copied renderer sources
will be overwritten, so first preserve any edits to those particular files.

To apply a reviewed plan, append `--apply`. Only the PDF installer preview needs Pillow:

```powershell
python -m pip install -r scripts/requirements-optional.txt
python scripts/sync_assets.py --code-root D:\code --apps command ghost vault crystalwire horizon pdf spaces --apply
```

The copier only updates source assets. It does not build or install apps, alter app
settings, edit shortcuts, change file associations or touch PDF document icons.
It also does not modify project metadata, manifests or source references automatically.

| App / project folder | Source consumers |
|---|---|
| Command / `axentra-command` | `tools/suite-icons/src/suite.rs` for runtime cards and `src/logo.rs`; `assets/command.ico` for packaging; local exporter files and exports also synchronized |
| Ghost / `axentra-ghost` | `src/suite_icon.rs` included by `src/logo.rs`; `assets/ghost.ico` |
| Vault / `axentra-vault` | `app/assets/app/vault-<size>.png`, `vault.ico`; runtime in `app/src/icon.rs`, chrome and floating session branding |
| CrystalWire / `axentra-crystalwire` | `assets/crystalwire-<size>.png`, `crystalwire.ico` |
| Horizon / `stillpoint` | `icons/16.png`, `32.png`, `48.png`, `128.png`, `512.png`; browser manifest and page favicons |
| PDF / `REA/axentra-pdf` | `packaging/icons/axentra-<size>.png`, `axentra.ico`; `axentra/ui/brand/` through 256 px; 96 px `packaging/windows/preview-app.bmp` |
| Spaces / `axentra-spaces` | `src/AxentraSpaces/Assets/spaces.ico`, root `spaces-512.png` |

## Axentra Mail

Mail is included in the kit as `assets/mail.ico` and `assets/mail-<size>.png`. Its
application repository and framework were not available when the icon was added, so
no automatic destination is assumed in `sync_assets.py`. Follow [MAIL.md](MAIL.md) to
wire the assets into the active Mail build. Add a sync mapping once its actual project
layout is known. Generating the icon alone does not register Mail in Command.

## Framework and installer wiring

**Rust applications:** either embed exported PNGs or include the shared `suite.rs` and
call `rgba("app-id", target_size)`. It has no external dependencies. Runtime and build
script copies must use the same renderer. Include the source in Cargo's build dependency
tracking, for example the existing Command `build.rs` line:

```rust
println!("cargo:rerun-if-changed=tools/suite-icons/src/suite.rs");
```

For Windows resources, use the exported ICO as the app's `ICON` resource. Command and
Ghost currently build ICO resources with their own wrappers, which can encode the
256 px frame as BMP instead of PNG. Container bytes can differ; compare decoded pixels
at every size with `verify_binary.py`.

**.NET/WPF:** set the project's `ApplicationIcon` to the exported ICO and rebuild/publish.
For Spaces, the source path is `Assets/spaces.ico` relative to its project. Ensure any
explicit window icon reference uses the same artwork. Updating a loose ICO next to an
old executable does not replace the embedded icon.

**Python/Qt/PyInstaller:** update both the ICO passed to PyInstaller and the runtime PNG
resources. Rebuild with `--clean` to avoid PyInstaller reusing old icon resources; run
the packaged app's existing self-test. The current PDF app also has an installer bitmap
preview. The copier recreates it from `pdf-256.png`, composited on white at 96×96 using
Pillow 12.3.0 and Lanczos resampling.

**Edge/Chrome extensions:** reference the exported PNGs in `manifest.json` and update
any explicit HTML favicon or toolbar image paths. The current Horizon page favicons
use `icons/32.png`; its manifest uses 16/32/48/128 px. Reload an unpacked extension from
the browser's Extensions page, then reopen or reload its pages.

**Inno Setup:** use the exported ICO for `SetupIconFile`, the application resource for
`UninstallDisplayIcon`, and a valid installed icon path for shortcuts. A standalone
installed ICO with a versioned path avoids stale executable icon-cache entries:

```ini
; Adapt filenames and paths to the product. These are illustrative entries.
[Setup]
SetupIconFile=assets\myapp.ico
UninstallDisplayIcon={app}\MyApp.exe

[Files]
Source: "assets\myapp.ico"; DestDir: "{app}\icons"; DestName: "myapp-glass-precision-v1.ico"

[Icons]
Name: "{autoprograms}\My App"; Filename: "{app}\MyApp.exe"; IconFilename: "{app}\icons\myapp-glass-precision-v1.ico"
```

For MSI/WiX or other packagers, supply the same ICO to installer/shortcut icon resources
and ensure it is installed when referenced by a file association. Do not use the icon
kit's temporary build directory as a shortcut target or installed icon location.

**Command app cards:** a new product needs a launcher registration as well as renderer
support. Update its icon asset reference and accent in the app registry. When replacing
served images at an unchanged URL, change the asset cache version or otherwise invalidate
the cached image. This prevents an updated executable from still displaying old cards.

## Verify what actually shipped

With the optional packages installed:

```powershell
python scripts/verify_binary.py "D:\build\MyApp.exe" assets/myapp.ico
```

The script compares RGBA pixels at every reference size in the executable's first
`RT_GROUP_ICON` resource. Use `--group-id NUMBER` if the intended icon is in another
resource group. Verify the final packaged/installed executable, not just the build
directory, because packagers can reuse stale output or another build can replace it.
Freeze the installer inputs or coordinate with other developers during a release.

Before release, verify:

- The existing app exports remain byte-identical, unless intentionally revised.
- Each app starts and shows its updated window/tray/in-app artwork.
- EXE, installer, Start menu and desktop shortcuts use the correct icon.
- Command cards load the current images; Horizon has been reloaded.
- PDF document icons and saved file-icon preferences remain unchanged.

For stale desktop shortcuts, preserve their executable target, arguments, working
directory and hotkey when changing only `IconLocation`. Point at the new installed ICO,
then notify the Windows shell or refresh its icons with `ie4uinit.exe -show`. Icon-cache
refresh alone cannot fix an old executable resource or incorrect shortcut reference.

## Keeping the kit current

After an approved change, commit the renderer, outputs, palette/design notes, compiler
record and checksum manifest together. Include a contact sheet. Prefer a new revision
name for a deliberate family-wide style change; keep older assets available through Git.
Never edit an export in a graphics editor without making the same change reproducible
in the source renderer.
