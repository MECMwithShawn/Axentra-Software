# Handoff brief for a developer or coding assistant

Copy the following request and replace the bracketed fields:

> Create an Axentra Glass + Precision icon for **[app name / lowercase app ID]**.
> The app does **[one-sentence purpose]**; the proposed symbol is **[symbol idea]**.
> Use `Axentra-Software/Icon-Kit` as the source of truth. Read its README, DESIGN,
> ADDING-AN-APP and INTEGRATION guides before editing.
> Horizon and CrystalWire use the current SVG masters described in OUTLINE-ICONS.md.
> Run `scripts/export.py` for the complete set; Cargo alone is not the current set.
> Preserve Archive's raster master and frame pixels as documented in ARCHIVE.md.
>
> Add the ID, accent and glyph to `renderer/src/suite.rs`. Reuse the existing tile,
> lighting, sampling and micro-size behavior. Preserve all existing app icons and
> the separate PDF document glass icon. Export the complete set; provide the new
> ICO, all PNG sizes and the expanded contact sheet. Verify existing per-app exports
> remain byte-identical before intentionally updating the approved manifest.
>
> Show the new icon at actual 16/24/32/48 px and 256 px on light and dark backgrounds.
> After approval, record the palette, symbol and compiler, update the kit's baseline,
> and wire the assets into **[target repository / framework]**. Rebuild and verify
> embedded executable resources; do not assume replacing a source PNG updates an
> installed application. Coordinate any app rebuild with concurrent development.

The kit includes the production source and outputs, so no image-generation account,
original chat, private repository access or local workstation files are required to
create matching artwork. App integration naturally needs access to that app's source.
No Claude-specific handoff is required; the same kit works for any developer.
