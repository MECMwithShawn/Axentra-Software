# Design specification

## Shared structure

Revision 1.3 uses [SVG masters](OUTLINE-ICONS.md) for Horizon and CrystalWire. The
procedural measurements below describe the six Rust icons and the historical filled
versions of those two apps. Use the SVG specification for their current artwork.

The family combines a dark glass tile, a precise central symbol and restrained colored
light. Keep the shared `sample()` and `rgba()` functions unchanged when adding an app.
New work normally needs only an app ID, an accent and a `glyph()` branch.

All dimensions below are **fractions of the full icon width**, using the renderer's
centered coordinate system `x,y = -0.5..0.5`, with positive y downward. `rbox()` takes
half-width and half-height, followed by corner radius.

| Element | Exact production definition |
|---|---|
| Outer tile | Center `(0,0)`, half-size `0.488`, radius `0.145` |
| Tile transparency | Outside the signed rounded-box distance `tile > 0` |
| Base vertical gradient | RGB float `[0.055,0.08,0.15]` to `[0.035,0.055,0.105]` |
| Accent edge | Outer `0.018` of the tile distance |
| Secondary inner edge | Distance `-0.040..=-0.035`, 22% of the top edge color, regular sizes only |
| Halo | Gaussian ring centered at `(0,-0.015)`, radius `0.23`, width `0.14` |
| Halo blend | 28% regular; 14% micro |
| Symbol rim | Four cardinal samples at offset `0.006`, 32% blend, regular sizes only |
| Symbol gradient | Accent mixed 36% toward white at the top, accent at the bottom |
| Highlight | Shared upper-left glint and glyph highlight; use `sample()` unchanged |
| Micro tier | Render sizes below 32 px, based on the **target size** |
| Sampling | 8×8 samples per pixel at sizes ≤48; 4×4 at larger sizes |
| Alpha | Accumulate premultiplied color, then write straight RGBA; clear transparent pixels |

Do not create a large icon and downscale it for the small exports. The micro tier removes
detail, fills Command's center and thickens some strokes. Each size must call `rgba()`.
Preserve negative-space details: Ghost's eyes, Vault's keyhole and CrystalWire's cuts.

## Product identities

| ID | Accent RGB / hex | Symbol and distinguishing details |
|---|---|---|
| `command` | `98,180,255` / `#62B4FF` | Hexagonal hub, six colored tiles linked by spokes |
| `ghost` | `170,126,255` / `#AA7EFF` | Familiar rounded head, two eyes, three scalloped lobes |
| `vault` | `64,139,255` / `#408BFF` | Icy-blue padlock, round keyhole, thin broken cyan circuit halo |
| `crystalwire` | `183,154,213` / `#B79AD5` | Lilac outlined crystal; SVG gradient and open facets |
| `horizon` | `255,180,70` / `#FFB446` | Gold outlined sunrise, five rays, three lines; SVG gradient |
| `pdf` | `255,105,119` / `#FF6977` | Coral folded document and dark angular Axentra mark |
| `spaces` | `26,211,192` / `#1AD3C0` | Two overlapping teal browser windows |
| `mail` | `239,136,199` / `#EF88C7` | Rose-pink closed envelope, recessed V-shaped flap and lower fold seams |

Ghost's body has its own violet-to-cyan gradient, float RGB `[0.68,0.45,1.0]` to
`[0.08,0.87,0.95]`; this is intentional. Command's core has an icy cyan-to-blue gradient;
its node order clockwise from the top is Vault, CrystalWire, Horizon, Spaces, PDF, Ghost.
Command retains its original six-node logo. Adding Mail as the eighth suite product
does not alter that logo or any existing icon. Mail uses the shared finish without
product-specific shader changes; exact geometry and integration notes are in [MAIL.md](MAIL.md).

The historical procedural Horizon uses a warmer top-edge blend and CrystalWire adds
light on the left facets. Current exports use the SVG finish described above.

Vault revision 1.1 adds `vault_lock()`, `vault_arc()` and `vault_finish()`. The common
tile and accent remain unchanged; the product-specific finish adds rings, dots, nodes
and an icy lock. Micro icons enlarge the lock and omit the inner/dotted rings. Exact
geometry and color values are documented in [VAULT.md](VAULT.md). This finish is specific
to Vault and must not be applied to the other app icons.

## New symbols

Use one recognizable silhouette, roughly within `x,y = -0.34..0.34`. Judge optical weight
next to Vault and Ghost, not just bounding-box dimensions. Favor the existing `rbox`,
`seg`, `len` and `poly` helpers. Avoid text, tiny badges, hairlines and decorative detail
that competes with the main symbol. Use `micro` to simplify features at 16/20/24 px.

Choose an accent distinct from the existing palette. Keep the common tile geometry,
rim, halo, gradient and sampling. This is Axentra artwork; do not copy Adobe's marks,
product abbreviations or brand palette. The inspiration is consistent geometric
discipline across a suite.

Review the exports on both light and dark backgrounds at actual 16, 24, 32 and 48 px,
as well as 256/512 px. A bright enlarged preview can hide weak small-size contrast.
