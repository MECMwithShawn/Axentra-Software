# Icons

- `apps/<colour>/axentra-onedrive-<n>.png`: the app icon at 16–512 px in the three colours
  offered in Settings > Appearance. **Ice** (`#B8DEFF`) is the Axentra Icon Kit's `onedrive`
  icon (Glass + Precision 1.6) and the default. **Periwinkle** (`#8B97FF`) and **azure**
  (`#48AAFF`) use the same glyph and renderer with only the accent changed.
  The installer copies the chosen colour into the hicolor theme as `axentra-onedrive`.
- `emblems/`: file-manager status badges (online-only, local, pinned, syncing, error, shared).

Regenerate the app icons from a checkout of the suite repository:

```bash
extras/icons/render-variants.sh /path/to/Axentra-Software
```

The script checks that ice still matches the kit's file byte for byte.
