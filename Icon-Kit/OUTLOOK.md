# Axentra Outlook icon

App ID `outlook`, accent `124,140,255` / `#7C8CFF` (periwinkle, between Vault's blue and
Ghost's violet). Added in revision 1.5. The symbol says "mail and calendar" in one mark: a
calendar page standing behind an envelope. It uses the shared `sample()` and `rgba()`
unchanged; the only product-specific code is the `glyph()` branch in
`renderer/src/suite.rs`.

All values are fractions of the icon width in the renderer's centred coordinates
(`x,y = -0.5..0.5`, y down). `rbox(x, y, cx, cy, half_w, half_h, radius)`.

| Part | Geometry |
|---|---|
| Envelope | `rbox(-0.075, 0.140, 0.235, 0.160, 0.035)` |
| Envelope flap | V cut: `seg(|x+0.075|, y-0.140, (0.222,-0.128) → (0,0.040)) <= cut` |
| Calendar page | `rbox(0.080, -0.045, 0.230, 0.205, 0.040)` |
| Binder tabs | two `rbox(0.080 ± 0.105, -0.263, 0.024, 0.050, 0.022)` |
| Header rule | horizontal cut at `y = -0.150`, half-thickness `cut/2 + 0.004`, within `|x-0.080| <= 0.205` |
| Date squares | `rbox` half-size `0.024 × 0.022`, radius `0.006`, at page offsets `(-0.080,-0.055) (0.005,-0.055) (0.090,-0.055) (0.175,-0.055) (0.175,0.025) (0.175,0.105)` |
| Gap | the page is removed where the envelope's distance is `<= 0.032` (`0.040` micro) |

`cut` is `0.014` at regular sizes and `0.024` in the micro tier (below 32 px). The micro tier
also drops the date squares, so 16/20/24 px icons keep a clean envelope, page and tabs.

Ready-to-use files: [Windows ICO](assets/outlook.ico), [512 px PNG](assets/outlook-512.png)
and the other PNG sizes in `assets/`. On Linux the app installs them as
`/usr/share/icons/hicolor/<n>x<n>/apps/axentra-outlook.png`.
