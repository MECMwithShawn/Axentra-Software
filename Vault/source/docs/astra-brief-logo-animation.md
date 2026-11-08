# Brief: Axentra Vault logo and startup animation

## Product
Axentra Vault is a desktop password manager for Windows: KeePass-compatible (.kdbx), with phone passkey unlock. It is built in Rust with the egui GUI toolkit; the animation is drawn in code, not played from a video. Audience: security-conscious individuals. Brand family: Axentra.

## Mood
Premium, cinematic and trustworthy. Deep navy darkness, electric-cyan light, a glass padlock with soft bloom, and concentric "circuit" rings made of solid, dashed and dotted arcs with glowing nodes at their ends. A faint horizon glow sits under the mark. References: a padlock in concentric neon circuit rings on deep blue, and a shield-and-padlock trace-in with an orbit ring. Avoid stock-photo clichés (binary rain, hooded hackers, globes), skeuomorphic metal safes and anything cartoonish.

## Palette (use exactly; tints are fine)
- Background: #050911 to #0c2038 to #1d5486 (radial, brightest at centre)
- Primary light: #22d3ee, with glow tint #5fe3ff and highlight #6fe8ff
- Glass body: #f2fdff to #a6e9ff (vertical gradient)
- Keyhole and ink: #0d3a66
- Text: #e7f6ff (wordmark), #7fbbcc (caption)

## Deliverables

### 1. App mark (logo), SVG
- Square, viewBox 0 0 256 256, on a rounded tile (radius about 56).
- Must read at 256, 64, 32 and 16 px. Supply a simplified 16/32 px variant if fine detail turns to mush at that size.
- Use only these SVG features: paths, circles, rects, linear and radial gradients, stroke-dasharray, and feGaussianBlur/feMerge glow. No embedded raster images, no fonts, no CSS animation inside the file.
- Also supply a flat one-colour version (cyan, transparent background) for small UI use.

### 2. Wordmark lockup, SVG
- "AXENTRA VAULT" in widely spaced capitals, with an optional caption "KEEPASS-COMPATIBLE · PASSKEY UNLOCK".
- Supply horizontal and stacked layouts. Convert text to outlines, or name a free font (OFL licence) we can embed.

### 3. Startup animation
About 2.0–2.6 s, then a slow ambient idle loop.

The default timing to improve on:

| Time | Beat |
|---|---|
| 0.0–0.4 s | Navy backdrop fades in, with a faint grid masked to a radial falloff |
| 0.3–1.2 s | Circuit rings trace in, staggered inner to outer, each turning at its own speed and direction; nodes light up when their arc completes |
| 1.0–1.5 s | Padlock forms; the shackle drops shut with a "click" (tiny scale bump and a bright flash) |
| 1.5–2.0 s | Bloom pulse ring expands from the lock; horizon glow brightens |
| 1.6–2.2 s | Wordmark and caption fade up (8 px rise) |
| Idle | Rings keep rotating very slowly (each 20–60 s per turn); nodes shimmer subtly |

Hard rules:
- **Never imply the vault is unlocked.** The sequence must end with the padlock closed. Unlocking happens only after the user signs in.
- **It must not block input.** The password form sits below the mark and is usable from frame 1, so the composition needs clear space beneath the mark.
- **It must respect reduced motion.** Provide a static end-frame for when animation is turned off.

Deliver the animation as:
- (a) a self-contained HTML/SVG/CSS prototype, and
- (b) a precise spec I can port to code:
  - every element's geometry, in the mark's 256 coordinate space
  - each element's start and end times and easing curve (cubic-bezier values)
  - each ring's dash pattern and rotation speed
  - glow radii and opacities

### 4. Optional extras
- A 1920×1080 still hero frame for a website or store page.
- A Windows .ico containing 16, 24, 32, 48 and 256 px images.

## Constraints and existing assets
- The current mark is attached as vault-mark.svg (padlock, four segmented rings, nodes, navy tile). Improve on it or replace it.
- The in-app toolbar icon set is a separate, simpler style (24×24 grid, 1.5 px #22d3ee line, rounded caps). The logo should feel related to it but can be richer.
- Text inside the app's own UI uses the default font. Don't rely on special Unicode symbols in the animation; draw everything as shapes.

## Please return
- SVG files
- the HTML prototype
- the timing/geometry spec (Markdown table)
- 2–3 alternative directions for the mark, each with a sentence of rationale. For example: padlock in rings (current), shield-and-keyhole, vault-door dial.
