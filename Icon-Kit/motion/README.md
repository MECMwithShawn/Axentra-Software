# Axentra orbital icon motion

An original animation of the eight canonical Glass + Precision icons, using
[ArtCraft's homepage](https://getartcraft.com/) as a visual motion reference.
Only Axentra's existing PNGs are loaded. This component has no runtime dependencies,
network services, external models, or Linux/Docker requirement.

## Preview

From the `Icon-Kit` directory:

```sh
python -m http.server 8000 --bind 127.0.0.1
```

Open `http://127.0.0.1:8000/motion/`. Hover an icon to lift and bounce it, click to spin,
or use Tab and Enter. The pause button stops the ambient orbit and automatic turns;
deliberate hover/click interaction remains available. Smaller screens use a readable
two-column arrangement. Reduced motion displays the icons without rotations or bends.

To make one portable HTML file, with the canonical images and code embedded:

```sh
python motion/build_preview.py
```

Open `out/axentra-motion-preview.html` directly in a WebGL-capable browser. The output
can be copied without any companion files. It is generated from the source, not an
additional image master. The existing PNG and ICO exports remain unchanged.

## Behavior and geometry

- Eight independently textured, double-sided 32 × 32 quad meshes.
- The same image/UVs remain on both faces. When viewed from the reverse side, the
  artwork is mirrored naturally by 3D rotation; it does not disappear or swap designs.
- A 0.045 radians/second orbital movement with depth, perspective, slight tilt,
  and staggered smooth turns on a 32-second cycle.
- True vertex deformation before rotation: `z = bend * (x*x - 0.12) + twist*x*y`.
  Bend strength varies with the turn angle and app index. Twist adds a gentle saddle
  shape. The tile's silhouette and image both curve, rather than rotating a flat CSS box.
- Hover uses a damped spring (stiffness 115, damping 11, integration steps ≤1/120 s),
  a small entrance impulse, 17% enlargement, and movement toward/above the viewer.
- Pointer position adds tilt. Clicking advances a spring-driven full rotation;
  multiple clicks are safe. App-specific phase/curvature gives each tile its own motion.
- Perspective depth testing resolves overlapping portions of a curved mesh. Entire
  cards are drawn from back to front, with their own depth buffer for self-occlusion.
- Canonical glyphs, palette and artwork are preserved. Lighting is restrained and
  applied only to the animated rendering.

The demo's rings and wordmark are presentation elements in `index.html`; the reusable
component in `axentra-motion.js` owns only the tiles and their interaction targets.

## Embed in the website or another web surface

Copy `motion.css`, `axentra-motion.js` and the eight `assets/*-256.png` files into the
target project's static assets. Keep the icons in an app-owned location. Load the CSS
and script, give the host a defined height, then mount:

```html
<link rel="stylesheet" href="/assets/motion/motion.css">
<div id="family-motion" class="axm-host" aria-label="Axentra app family"></div>
<script src="/assets/motion/axentra-motion.js"></script>
<script>
AxentraMotion.mount(document.getElementById('family-motion'), {
  assetBase: '/assets/icons/glass-precision',
  onSelect: ({id, name}) => { /* application-specific action */ }
}).then(controller => {
  // controller.setPaused(true);       Stop ambient movement.
  // controller.seek(15);              Deterministic preview time in seconds.
  // controller.getState();            Inspect renderer / loaded assets / poses.
  // controller.destroy();             Remove listeners, observers and GPU resources.
});
</script>
```

`assetBase` must contain the exact filenames `command-256.png`, `ghost-256.png`,
`vault-256.png`, `crystalwire-256.png`, `horizon-256.png`, `pdf-256.png`, `spaces-256.png`,
and `mail-256.png`. Prefer same-origin assets. Alternatively pass an `images` object
mapping the app IDs to embedded data URLs, as the portable preview does.

Use CSS variables `--axm-foreground` and `--axm-label-bg` for label colors in the host
site's theme. On narrow viewports allow at least 820 px of height for the two-column
layout, plus any surrounding header/copy. Retain keyboard focus styles and accessible
button names. The sample page uses no external fonts.

The component supports reduced motion, pauses rendering while hidden/offscreen, and
falls back to static image buttons when WebGL is unavailable. The desktop shortcut
and executable ICOs stay static; this animation is for web/app presentation surfaces.

Website integration is a separate change: the active Axentra website checkout was
not edited or deployed by this icon-kit addition. Its current orbit implementation
can be replaced with this component after wiring the site's theme and product links.

## Validation

The accompanying `check.cjs` uses Playwright for optional browser checks. It covers
all eight textures, spring overshoot, deformation during a spin, keyboard activation,
390 px layout, reduced motion, and WebGL-unavailable fallback. It launches an isolated
headless browser and writes screenshots/results to `out/`.

```sh
npm install --no-save playwright
python motion/build_preview.py
node motion/check.cjs
```

The test defaults to installed Edge on Windows. For a Linux/Chromium test, install
Playwright's Chromium (`npx playwright install chromium`), then run with
`AXENTRA_BROWSER=chromium`. This dependency is for testing only; the animation runs
in a standard browser without Node, Python or Playwright.
