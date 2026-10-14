# Adding an app without changing the existing icons

Work in `renderer/src/suite.rs`. The following example adds a sample app named `signal`.
It is a recipe, not a production icon. Use your real app ID and symbol instead.

## 1. Register the app

Append `"signal"` to `APPS`. App IDs become output filenames. Keep existing IDs stable.

## 2. Add the accent

Before the fallback arm in `accent()`:

```rust
"signal" => [168, 219, 100], // #A8DB64; example only, distinct from the current palette
```

## 3. Add the silhouette

Before the fallback arm in `glyph()`:

```rust
"signal" => {
    let half_width = if micro { 0.042 } else { 0.032 };
    [(-0.19, 0.14), (0.0, 0.29), (0.19, 0.21)]
        .iter()
        .any(|&(cx, half_height)| {
            rbox(x, y, cx, 0.0, half_width, half_height, half_width) <= 0.0
        })
}
```

The shared renderer supplies the tile, edge, halo, color ramp, highlights and alpha.
Do not change `sample()` or `rgba()` to style a new app unless a deliberate family-wide
revision has been approved. No changes are needed in the encoder or preview loop.

## 4. Render and review

```powershell
cargo run --manifest-path renderer/Cargo.toml --release --locked --offline -- out
```

Expect `signal.ico`, eight `signal-<size>.png` exports and an expanded `out/preview.png`.
Review actual small-size PNGs, each ICO frame, the full contact sheet and light/dark
backgrounds. Check that unrelated icons are untouched before updating the baseline:

```powershell
python scripts/verify.py --rendered out --existing-only
```

This mode intentionally checks only existing per-app exports. It allows the contact
sheet to expand, and it does **not** certify the new app or approve source changes.
After visual approval, copy the new app's files and updated contact sheet to `assets/`.
Keep the existing files if they already match. Add the app to the table in `DESIGN.md`,
record the change and compiler in `build-info.json`, and update integration mappings
for any new target project.

Then deliberately record the approved source/assets baseline and verify it:

```powershell
python scripts/verify.py --write-manifest
python scripts/verify.py --rendered out
```

Review the manifest diff. New app files and the contact sheet are expected to change;
old app PNG/ICO hashes should stay unchanged. Never rebaseline merely to hide a failed
regeneration check. If only an environment changed, use the original toolchain/target
or distribute the existing committed files.

## 5. Integrate

Use [INTEGRATION.md](INTEGRATION.md) to update executable, in-app, tray, installer,
shortcut and launcher references. Register the new app with Command separately; the
renderer list does not automatically add a product to Command's application registry.

Keep app icons and document file-type icons distinct. Adding a product must never
change the glass icon associated with existing PDF documents.
