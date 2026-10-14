# Axentra Mail icon and build handoff

Axentra Mail is an Outlook replacement in development, intended to provide automatic
spam handling, inbox organization and local cleanup using Ollama/Qwen. The icon uses
one clear envelope silhouette so the app remains identifiable at taskbar and tray sizes.

## Identity and source

- App ID: `mail`; accent: rose pink, RGB `239,136,199`, hex `#EF88C7`.
- Source of truth: the `mail` entries in `renderer/src/suite.rs`.
- Shared dark glass tile, border, gradient, glow, highlights and sampling are unchanged.
- Envelope: `rbox(x,y,0,0,0.315,0.215,0.045)` in centered normalized coordinates.
- Flap seam: mirrored segment from `(0.300,-0.178)` to `(0,0.040)`, half-width `0.014`.
- Lower fold seams: mirrored segment from `(0.306,0.193)` to `(0.125,-0.012)`, half-width `0.009`.
- Below 32 px: flap half-width increases to `0.024`; lower seams are omitted.
- Transparent corners remain outside the shared tile. No letters, fonts or AI badges.

## Files for the current build

| File | Use |
|---|---|
| `assets/mail.ico` | Windows executable resource, installer and installed shortcut icon |
| `assets/mail-16.png`, `mail-24.png`, `mail-32.png` | Small app/window/tray surfaces |
| `assets/mail-48.png`, `mail-64.png`, `mail-128.png` | Launcher and in-app branding |
| `assets/mail-256.png`, `mail-512.png` | Larger branding surfaces and previews |
| `references/mail-size-proof.png` | Actual-size samples on light/dark backgrounds |

The ICO contains independently rendered 16, 20, 24, 32, 40, 48, 64 and 256 px frames.
Use the appropriate supplied PNG for each size rather than downscaling the 512 px art
into small slots. The kit's normal generation and verification commands reproduce Mail
along with the rest of the family; see [README.md](README.md).

## Developer handoff

Copy `assets/mail.ico` and `assets/mail-*.png` into the Mail project's branding assets.
Wire the ICO into the executable's Windows icon resource and the installer, and wire
the PNGs into window, tray and in-app branding. Framework-specific wiring and executable
verification are covered in [INTEGRATION.md](INTEGRATION.md).

Install a persistent standalone copy as `icons/mail-glass-precision-v1.ico`. Explicitly
point desktop and Start menu shortcuts at that installed path. Keep the executable
target and arguments separate from the icon path. Updating an icon embedded at the
same executable path can leave Windows displaying a cached image; the standalone
versioned filename avoids the problem previously encountered with Command.

For example, adapt these Inno Setup entries to the Mail project's actual paths:

```ini
[Files]
Source: "assets\mail.ico"; DestDir: "{app}\icons"; DestName: "mail-glass-precision-v1.ico"

[Icons]
Name: "{autoprograms}\Axentra Mail"; Filename: "{app}\AxentraMail.exe"; IconFilename: "{app}\icons\mail-glass-precision-v1.ico"
Name: "{autodesktop}\Axentra Mail"; Filename: "{app}\AxentraMail.exe"; IconFilename: "{app}\icons\mail-glass-precision-v1.ico"
```

No Mail app checkout was available when this asset was created. The kit therefore
does not claim to have wired, built or installed the Mail application. Add the actual
project mapping to `scripts/sync_assets.py` once its layout is established. A Command
launcher entry is a separate app-registration change; use the `mail` icon and `#EF88C7`
accent for that entry, and update the copied renderer when Command serves it.

All 63 exports for the existing seven apps are byte-identical to revision 1.1. The
glass PDF document icon is separate and unchanged. Mail's clean-envelope symbol is
original procedural geometry and does not use Outlook or Adobe brand artwork.
