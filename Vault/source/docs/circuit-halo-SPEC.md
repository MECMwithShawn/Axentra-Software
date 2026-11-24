# Circuit Halo — reference-inspired revision

This revision develops Circuit around the supplied visual reference: a small closed lock, many fine concentric arcs, a faint triangulated network, luminous nodes, and a floating horizon light. The supplied Axentra palette is retained. The artwork is native vector geometry; the reference image is not embedded, traced as a raster, or required at runtime.

Open **prototype.html** to play the 2.40-second entrance, pause, scrub, toggle the app tile, or show the reduced-motion end-frame. The full startup composition is untiled. A tiled app icon, optical size variants, and Windows ICO are supplied separately.

## Geometry

All positions are in the mark's 256 × 256 coordinate system. Circuit centre C=(128,118). Positive angles rotate clockwise; 0° points right. A point at angle a and radius r is `(128+r cos(aπ/180),118+r sin(aπ/180))`. Scale all coordinates, strokes, dash lengths and Gaussian sigmas by renderedSize/256.

`geometry.json` contains every ring radius, arc start/end angle, stroke width, alpha, dash pattern, node attachment, rotation period, entrance phase, all 134 mesh paths, mesh-node coordinates, branch paths, and exact lock path data. It is the numeric companion to this specification. The SVGs preserve exact paint order, colours and gradients.

| Layer | Exact geometry and paint |
|---|---|
| Optional tile | Rect(4,4),248×248,rx56. Radial gradient centre(50%,45%),radius65%; `#1d5486` at0, `#0c2038` at.58, `#050911` at1 |
| Tile rim | Rect(4.5,4.5),247×247,rx55.5; stroke .7 `#5fe3ff`,alpha .18 |
| Atmosphere | Circle C,r125; radial `#1d5486`,alpha .9 at0,.42 at.54,0 at1 |
| Central bloom | Circle(128,119),r64; radial `#22d3ee`,alpha .28 at0,.08 at.45,0 at1 |
| Mesh | 134 straight segments from deterministic irregular bands with 23,18,13,7 vertices near radii104,83,60,31; exact segments in JSON. Stroke `#22d3ee`, widths .22 or .30, alpha .09–.17 |
| Mesh junctions | Every fourth vertex in flattened bands; radius .5, `#5fe3ff`,alpha .3 |
| Circuit branches | Eight angled paths in JSON; `#6fe8ff`,width .4,alpha .55 |
| Shackle | `M119 118V108C119 94 137 94 137 108V118`; stroke3.5, round caps, vertical `#f2fdff`→`#a6e9ff` |
| Body | Rect-equivalent path(112,116),32×26,corner radius3.2; vertical `#f2fdff`→`#a6e9ff` |
| Lock glow | Duplicate body path `#22d3ee`,alpha .30,Gaussian sigma3.8 |
| Keyhole | `M128 120.5A3.9 3.9 0 0 1 129.6 127.96V135Q129.6 136.6 128 136.6Q126.4 136.6 126.4 135V127.96A3.9 3.9 0 0 1 128 120.5Z`; fill `#0d3a66` |
| Seal pulse | Circle C, radius24→108; stroke .9 `#5fe3ff`,peak alpha .18 |
| Horizon bloom | Elliptical circle centred(128,240),radius52 horizontally and4.472 vertically. Radial stops: `#f2fdff` alpha .94 at0, `#6fe8ff` alpha .65 at.12, `#5fe3ff` alpha .18 at.45, `#22d3ee` alpha0 at1 |
| Horizon core | `M117 240H139`; stroke .6 `#6fe8ff`,alpha .65, node glow filter |

The full body width is 32/210 of the outer circuit diameter, approximately **15%**, preserving the reference's small centrepiece. The lock is closed for every visible frame. Even at its maximum −.8-unit seating offset, the shackle endpoint y117.2 overlaps the body beginning at y116.

## Ring system

Solid rings use round caps. Dotted rings use dasharray `.01 2.55`, also round caps, for a 2.56-unit pitch. Stroke colour is `#6fe8ff` when width>.6; otherwise `#5fe3ff`.

| ID | Radius | Width | Base alpha | Pattern | Idle revolution | Direction |
|---:|---:|---:|---:|---|---:|---|
| 0 | 38 | .55 | .72 | Solid | 40 s | CW |
| 1 | 42.7 | .86 | .94 | Dotted | 44 s | CCW |
| 2 | 47.8 | .42 | .54 | Solid | 48 s | CW |
| 3 | 52.9 | .87 | .91 | Solid | 52 s | CCW |
| 4 | 58.2 | .34 | .55 | Solid | 60 s | CW |
| 5 | 63.4 | .84 | .84 | Dotted | 56 s | CCW |
| 6 | 68.6 | .43 | .61 | Solid | 48 s | CW |
| 7 | 73.8 | .34 | .42 | Solid | 60 s | CCW |
| 8 | 79 | .88 | .92 | Solid | 44 s | CW |
| 9 | 84.2 | .82 | .78 | Dotted | 52 s | CCW |
| 10 | 89.4 | .34 | .49 | Solid | 60 s | CW |
| 11 | 94.6 | .55 | .67 | Solid | 56 s | CCW |
| 12 | 99.8 | .90 | .94 | Dotted | 48 s | CW |
| 13 | 105 | .74 | .81 | Solid | 60 s | CCW |

The `arcs` array lists every disconnected arc in degrees. Trace each arc by drawing from a to `a+(b−a)p`, where p is that ring's eased reveal fraction. Keep the dash pattern fixed; do not use a reveal dash that replaces the dot pattern. Hide degenerate paths at p=0.

Each ring's `nodes` array identifies arc indices. `nodeEnds[k]` selects that arc's first angle(0) or last angle(1). These 13 nodes rotate with their parent rings. Radius is1.20 for rings above radius90, otherwise1.02. Alternate nodes in each ring use `#f2fdff` and `#6fe8ff` respectively. They light only after their parent arc is complete.

## Easing and timeline

Let E(t,a,b,curve) be 0 before a, 1 after b, and the CSS cubic-bezier value at elapsed fraction `(t−a)/(b−a)` between them. Invert the Bezier x polynomial to obtain y. The prototype uses 18 bisection iterations.

| Curve | Cubic-bezier |
|---|---|
| Reveal | (.22,1,.36,1) |
| Settle | (.4,0,.2,1) |
| Soft | (.25,.1,.25,1) |

| Element | Start→end seconds | Change | Easing |
|---|---|---|---|
| Optional tile | 0→.42 | Alpha0→1 when enabled | Reveal |
| Atmosphere | 0→.48 | Alpha0→1 | Reveal |
| Mesh | .12→.95 | Group alpha0→1 | Reveal |
| Ring i | `.16+.026i` → `.88+.043i` | Arc reveal0→1 | Reveal |
| Ring initial rotation | Ring start→1.50 | `(−1 for even i, +1 for odd i)×(5+.32i)` degrees→0° | Reveal |
| Branch paths | .45→1.24 | Group alpha0→1 | Reveal |
| Nodes | Ring end→ring end+.12 | Alpha0→1 | Soft |
| Closed lock | .92→1.30 | Group alpha0→1 | Reveal |
| Shackle seating | 1.18→1.42 | Local y−.8→0 | Settle |
| Lock compression | 1.42→1.49 | Scale1→.984 about(128,120) | Settle |
| Lock settle | 1.49→1.62 | Scale.984→1 | Settle |
| Pulse radius | 1.46→2.12 | Radius24→108 | Reveal |
| Pulse alpha | 1.46→1.57→2.12 | 0→.18→0 | Soft |
| Horizon first lift | .20→.80 | Group alpha0→.20 | Reveal |
| Horizon second lift | 1.45→2.10 | Group alpha.20→1 | Reveal |
| Wordmark | 1.64→2.20 | Alpha0→1; y+8→0 UI pixels | Reveal |
| Caption | 1.76→2.32 | Alpha0→1; y+8→0 UI pixels | Reveal |
| Final hold | 2.32→2.40 | Completed composition | Constant |
| Idle | 2.40 onward | Ring rotations; subtle node shimmer | Linear rotation |

Pulse alpha is `.18 E(t,1.46,1.57,Soft) × (1−E(t,1.57,2.12,Soft))`, forced0 outside[1.46,2.12]. Horizon group alpha is `.2E(t,.2,.8,Reveal)+.8E(t,1.45,2.1,Reveal)`.

For idle, u=max(0,t−2.4). Ring angle is `initial×(1−E(t,start,1.5,Reveal)) + dir×360u/period`. Its node k alpha is multiplied by `1+E(t,2.4,3.2,Soft)×(−.05+.05cos(2πu/(5+.2i)+k+.7i))`, a smooth shimmer between .9 and1. The mesh, lock, atmosphere and horizon do not rotate.

## Glow and layout

Node glow is one Gaussian sigma1.4, merged with the source. Lock glow is Gaussian sigma3.8 only. Both use sRGB interpolation and user-space filter bounds(0,0,256,256). Rings remain crisp; broad glow is provided by the atmosphere, rather than blurring every line.

The desktop prototype gives the untiled emblem a392×392 CSS-pixel artboard, followed by the outlined wordmark(max346×27) and caption(max310×9). The entire identity region reserves455 px from frame zero. It scales down at narrower viewports. Text rises are paint transforms only, so the form never shifts or waits for animation. In native egui, scale mark geometry from the same coordinate space and allocate the entire identity region independently of elapsed time.

Reduced motion immediately samples t2.4 with idle time0 and stops animation scheduling. This also applies when the OS preference changes during playback. The prototype's checkbox cannot override an OS request. Input is outside the animated region and always enabled. The demo form performs no real authentication and sends or saves no credentials. Hidden tabs pause their animation clock.

## Asset selection

| File | Intended use |
|---|---|
| `assets/circuit-halo-emblem.svg` | Untiled full startup mark; static end-frame; ideally128 px or larger |
| `assets/circuit-halo-icon.svg` | Full tiled mark,128–256 px |
| `assets/circuit-halo-medium.svg` | Four retained tracks, no mesh, thicker strokes, lock scaled1.3;48–64 px |
| `assets/circuit-halo-small.svg` | One interrupted perimeter and enlarged lock;24–32 px |
| `assets/circuit-halo-micro.svg` | Enlarged closed lock without rings;16 px |
| `assets/circuit-halo-flat.svg` | Transparent single-colour cyan,48 px and larger |
| `assets/circuit-halo-flat-micro.svg` | Transparent single-colour cyan,16–32 px |
| `assets/circuit-halo-lockup.svg` | Outlined stacked lockup,440×348 |
| `assets/circuit-halo-horizontal.svg` | Outlined horizontal lockup |
| `assets/circuit-halo-hero.svg` | 1920×1080 hero; raster version in `preview/hero.png` |
| `assets/circuit-halo.ico` | Windows icon with16,24,32,48,256 px PNG entries |

Small and flat variants intentionally reuse the simplified Circuit silhouettes from the original package. Detailed mesh is never squeezed into16–32 px. The dense emblem is new code-drawn vector artwork.

Source: `../source/build-circuit-halo.mjs` and `../source/circuit-halo.template.html`. Run the builder with Node after the original package exists. `../source/check-circuit-halo.mjs` uses Sharp and Playwright to regenerate PNGs, ICO, screenshots, and the verification report. The Rust application is not modified by this design package.
