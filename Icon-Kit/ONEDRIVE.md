# Axentra OneDrive icon

App ID `onedrive`, accent `184,222,255` / `#B8DEFF` (ice: lighter than Command's sky blue
and Vault's royal blue). Added in revision 1.6. The symbol is a single cloud. It uses the
shared `sample()` and `rgba()` unchanged; the only product-specific code is the `glyph()`
branch in `renderer/src/suite.rs`.

All values are fractions of the icon width in the renderer's centred coordinates
(`x,y = -0.5..0.5`, y down), after the cloud's own transform
`(x, y) = (x / 1.15, (y - 0.02) / 1.15)`, which enlarges it by 15% and lowers it slightly.

| Part | Geometry |
|---|---|
| Left lobe | circle at `(-0.13, 0.05)`, radius `0.11` |
| Right lobe | circle at `(0.13, 0.05)`, radius `0.11` |
| Top lobe | circle at `(0, -0.04)`, radius `0.155` |
| Base | `rbox(0, 0.105, 0.13, 0.055, 0)`, joining the lobes into a flat bottom |

The cloud is a solid silhouette, so the micro tier (below 32 px) uses the same shape.

Ready-to-use files: [Windows ICO](assets/onedrive.ico), [512 px PNG](assets/onedrive-512.png)
and the other PNG sizes in `assets/`. On Linux the app installs them as
`~/.local/share/icons/hicolor/<n>x<n>/apps/axentra-onedrive.png`.

The app also offers periwinkle and azure versions of this icon in its settings. They are
rendered from this same glyph with a different accent and ship with the app's source; ice,
in this kit, is the default and the suite's reference icon.
