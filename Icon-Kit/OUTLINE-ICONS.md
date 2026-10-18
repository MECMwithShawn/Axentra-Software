# Horizon and CrystalWire outline icons — revision 1.3

These are the approved application icons adopted by the website on October 17, 2026,
and synchronized into this public kit on October 18. All eight PNG sizes and both ICOs
are copied unchanged from the application exports; a full fresh export reproduces
them byte for byte. This update does not publish new application releases.

| App | Editable master | Identity |
|---|---|---|
| Horizon | [`svg/horizon.svg`](svg/horizon.svg) | Outlined gold sunrise, five rays and three horizontal lines |
| CrystalWire | [`svg/crystalwire.svg`](svg/crystalwire.svg) | Lilac outlined crystal with open facets |

Both masters use a 64 × 64 viewBox, a rounded dark glass tile with a gradient rim,
restrained halo and unfilled linework. Horizon's line gradient is `#FFF0C9` →
`#E6C783` → `#BA934E`. CrystalWire's line gradient is `#E4D7FF` → `#A58AC9`;
its presentation accent is `#B79AD5`.

## Reproduction

Run the complete recipe in [README.md](README.md), using `scripts/export.py`.
The Rust pass preserves the other six icons, then `svg/render.cjs` replaces the two
historical filled exports and their contact-sheet columns. Do not distribute the
intermediate Cargo-only output as the current family.

The SVG exporter uses the same rules as the product-local `tools/make-icon.cjs`
generators: Sharp 0.35.5, density 384, resize independently to each target size,
standard RGBA PNG output. It does not downscale the 512 px export for smaller sizes.
At 16/24 px, Horizon's 1.65 stroke becomes 2.2. CrystalWire's 1.3 symbol stroke
becomes 2.2 at 16 px and 1.7 at 24 px; its 1.4 rim becomes 2 at both sizes.
`svg/horizon-16.svg` is the matching small-size SVG used by the application.

Each Windows ICO preserves the application's seven frames: 16, 24, 32, 48, 64, 128
and 256 px. Frames below 256 use 32-bit BGRA DIBs with an AND mask; the 256 frame is
PNG. The verifier checks these sizes explicitly, while keeping the original
eight-frame requirement for the six Rust icons.

`build-info.json` records the exact rendering environment. Reproduction on another
native SVG library/toolchain must pass `verify.py --rendered out` before claiming
byte identity. Ready-to-use assets require no renderer installation.

## Integration

- Horizon: all `icons/<size>.png`, `icons/horizon.ico`, both SVGs and
  `icons/apps/horizon.png` (the 128 px export).
- CrystalWire: `assets/crystalwire-<size>.png`, `assets/crystalwire.ico` and the SVG.
- Command: `assets/horizon-outline.svg` and `assets/crystalwire-outline.svg`, plus
  the current suite exports. Its card routes already embed those SVGs.
- Website: copy the exact 32/256 px PNGs, with new asset URLs or cache versions.

The sync script previews these destinations before `--apply`. It preserves the
product-local generators and does not rebuild or install applications. Command's
legacy procedural Horizon/CrystalWire branches remain historical compatibility
code; use the SVGs or approved PNGs for those two cards. The other six app exports
and every PDF document-icon file remain unchanged.
