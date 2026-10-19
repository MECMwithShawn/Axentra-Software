# Icon kit revisions

## Glass + Precision 1.6 — 2026-10-19

- Add Axentra OneDrive as the eleventh app: an ice (`#B8DEFF`) cloud, using the shared
  tile, rim, halo, gradient and sampling unchanged. Geometry is in [ONEDRIVE.md](ONEDRIVE.md).
- Rendered with rustc 1.99.1 on x86_64 Linux. The seven existing procedural icons
  (Command, Ghost, Vault, PDF, Spaces, Mail, Outlook) reproduce byte for byte; no existing
  PNG or ICO changed.
- `assets/preview.png` gains an eleventh column, rendered alone with the renderer's own
  `preview()` and appended without re-rendering the other ten. As a control, Outlook's
  column rendered the same way matched the existing sheet exactly.
  `references/production-preview.png` and the motion component are unchanged.

## Glass + Precision 1.5 — 2026-10-19

- Add Axentra Outlook as the tenth app: a periwinkle (`#7C8CFF`) calendar page behind
  an envelope, using the shared tile, rim, halo, gradient and sampling unchanged.
  Geometry and micro-tier rules are in [OUTLOOK.md](OUTLOOK.md).
- Rendered with rustc 1.99.1 on x86_64 Linux. The six existing procedural icons
  (Command, Ghost, Vault, PDF, Spaces, Mail) reproduce byte for byte from this
  toolchain; no existing PNG or ICO changed.
- `assets/preview.png` gains a tenth column, drawn with the renderer's own `rgba()`
  and blend and appended without re-rendering the other nine (the Mail column was
  re-rendered as a control and matched exactly). `references/production-preview.png`
  and the motion component still show nine apps until the complete exporter is rerun.
- `build-info.json` app order and manifest baseline updated (126 files, 10 apps).

## Glass + Precision 1.4 — 2026-10-18

- Add Archive as the ninth app, preserving its installed eight-frame Windows ICO.
- Export native-frame PNGs and documented 128/512 px derivatives without redrawing.
- Expand the production preview and motion component to nine apps, including a
  centered fifth row on phones, keyboard activation and static fallback.
- Add the Command Archive ICO mapping, provenance and checksum baseline.
- Preserve all 72 existing app exports and every PDF document-icon file byte for byte.

## Glass + Precision 1.3 — 2026-10-18

- Synchronize Horizon's gold outlined sunrise and CrystalWire's lilac outlined crystal.
- Preserve the exact eight PNG sizes and seven-frame ICO from each application.
- Add editable SVG masters and a complete Rust + SVG export command.
- Refresh the production preview, motion accent, integration mappings and checksums.
- Preserve the other 54 app exports and every PDF document-icon file byte for byte.

## Motion 1 — 2026-10-14

- Add an eight-app orbital preview and reusable WebGL component.
- Use spring hover bounce, pointer tilt, true curved-mesh spins and mirrored backs.
- Include accessible buttons, reduced motion, narrow-screen layout and static fallback.
- Add a portable HTML builder and optional isolated-browser interaction checks.
- Preserve all current app and PDF document artwork byte for byte.

## Glass + Precision 1.2 — 2026-10-14

- Add Axentra Mail with a rose-pink envelope and the existing suite tile and finish.
- Render a stronger flap seam and omit lower fold seams below 32 px.
- Supply eight PNG sizes, an eight-frame Windows ICO, and Mail build handoff notes.
- Verify all 63 existing app exports remain byte-identical.
- Preserve both PDF document ICOs and the shared renderer finish unchanged.
- Expand the production contact sheet and include a light/dark optical-size proof.

## Glass + Precision 1.1 — 2026-10-14

- Restore Vault's circuit-halo identity within the suite's existing blue glass tile.
- Use a smaller icy-blue lock, thin broken cyan rings, sparse dots and luminous nodes.
- Simplify the halo and enlarge the lock below 32 px.
- Preserve all 54 PNG/ICO exports for the other six apps byte for byte.
- Refresh the production contact sheet and record the exact Vault geometry/reference.
- Preserve the glass PDF document icon and its saved alternate unchanged.

## Glass + Precision 1 — 2026-10-14

- Publish the seven-app source renderer, exports, design guide, checksum baseline,
  new-app example and integration tools.
