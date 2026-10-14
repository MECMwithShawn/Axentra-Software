# PDF document icons — preserved separately

The current PDF **document** icon is `pdf-file-glossy.ico`: the dark glass tile with a
white page outline and teal PDF label. It is intentionally different from the coral
Axentra PDF **application** icon in `../assets/pdf.ico`.

The original flat alternate is retained for existing users who selected it. New
installations use glass; upgrades should preserve the user's saved choice.

| Asset | SHA-256 |
|---|---|
| `pdf-file-glossy.ico` | `6b4768aa457dd9f93e398fa14e80e9cf7a762e1526f348aad4362d2530b4a7f2` |
| `pdf-file-flat.ico` | `b4ad4b89edf8b2b4ce85cb6c1af091a19a39e5d1157e2259124e27f23e2df586` |

`source/` preserves the original light/dark SVG design sources for both variants.
They contain live PDF text and refer to `DejaVu Sans,Arial,sans-serif`. The original
SVG-to-ICO export toolchain and font version were not recorded, so rerendering these
SVGs is **not guaranteed to reproduce the legacy ICO pixels**. Reuse the committed
ICO for an exact match. The app icon renderer does not generate or modify these files.

For Windows, register the document ICO under the app-specific
`AxentraPDF.Document\DefaultIcon` using the path where the installer actually places
the file. Do not repoint it to the application executable or force a default PDF
handler. The current app installer preserves file-icon choices on upgrade.
