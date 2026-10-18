# Vault: circuit halo within the suite

![Current Vault production icon](assets/vault-512.png)

The October 14, 2026 revision follows the original
[circuit-halo reference](references/vault-circuit-reference.png), while matching the
installed Glass + Precision family. It uses the **same navy tile, rounded corners,
blue rim, inner edge, highlight and sampling** as the other icons. A smaller ice-blue
padlock sits inside thin broken cyan rings; the halo has enough open space to remain
legible on Command's cards and desktop shortcuts.

This is fully procedural artwork. The reference PNG is not embedded or resampled by
the renderer. Reproducing this design requires no image-generation service, raster
master, fonts or additional dependencies.

## Source

`renderer/src/suite.rs` contains three Vault-specific helpers:

- `vault_lock()` gives a signed distance to the lock and identifies the keyhole.
- `vault_arc()` measures round-capped arc distance.
- `vault_finish()` adds the lock, rings, nodes and restrained glow to the shared tile.

The Vault branch in `sample()` calls this finish after the common tile has been drawn.
All other app rendering paths are unchanged. Vault keeps `#408BFF` as its product accent,
so Command's blue Vault node and card accent remain stable.

## Exact dimensions and colors

Coordinates use the suite's full-width units: centered `-0.5..0.5`, y downward.
Angles are degrees clockwise from the right-hand horizontal axis.

| Component | Definition |
|---|---|
| Lock body | Center `(0,0.069)`; half-width `0.145`, half-height `0.108`, corner radius `0.032` |
| Shackle | Center `(0,-0.075)`, radius `0.083`, half-stroke `0.020`; legs join at y `-0.020` |
| Keyhole | Circle center `(0,0.034)`, radius `0.027`; rounded stem center `(0,0.073)`, half-size `(0.013,0.033)` |
| Outer arcs | Radius `0.355`; angles `-110..86`, `104..137`, `151..223` |
| Inner arcs | Radius `0.273`; angles `-133..-27`, `-10..138`, `160..206` |
| Solid stroke | Half-width `0.0055`; inner arcs reduced by `0.0015` |
| Dotted ring | Radius `0.315`, 48 dots, dot radius `0.0028`, 60% color blend |
| Nodes | Outer ring at `86`, `137`, `223` degrees; inner ring at `138` degrees; radius `0.013` |
| Cyan tracks | Float RGB `[0.24,0.79,1.0]`, mixed toward `[0.60,0.94,1.0]` at the top |
| Lock gradient | Float RGB `[0.73,0.96,1.0]` to `[0.32,0.68,0.98]` |
| Keyhole color | Float RGB `[0.035,0.16,0.29]` |
| Ring glow | Gaussian distance width `0.018`, blend 23% |
| Lock glow | Gaussian distance width `0.021`, blend 22% |

At sizes below 32 px, the lock is enlarged by 16%, the keyhole radius becomes `0.032`,
the outer stroke half-width becomes `0.009`, and inner/dotted rings are omitted. Ring
glow drops to 8%; nodes use radius `0.012`. Every size is rendered independently.

The canonical outputs remain `assets/vault-<size>.png` and `assets/vault.ico`. The ICO
contains the original eight Rust frame sizes. Horizon and CrystalWire use seven SVG
frame sizes as of revision 1.3. Run the complete `scripts/export.py` exporter
and `scripts/verify.py --rendered out` to reproduce the approved bytes.

## App and shortcut integration

Vault embeds the PNGs for window, tray and in-app branding, and the ICO for its Windows
resource. Command embeds this renderer for its Vault card, so it also needs rebuilding
after a Vault geometry change. Update Command's icon cache-version token when shipping.

Vault's Inno installer now installs `icons/vault-circuit-v2.ico` alongside the executable
and assigns that explicit icon to its Start menu and desktop shortcuts. A fresh icon path
prevents the previous executable or shortcut icon from lingering in Windows' cache.
PDF document icons and the other suite app assets are unaffected.
