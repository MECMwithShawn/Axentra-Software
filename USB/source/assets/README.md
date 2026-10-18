# Axentra USB icon

`axentra-usb.svg` is the editable master: mint USB linework, dark glass tile and luminous outer rim.
The PNGs cover 16–1024 px; the ICO includes 16, 24, 32, 48, 64, 128 and 256 px.
The smallest two sizes use slightly stronger linework for readability.

To regenerate with Node.js and the `sharp` package available: `node assets/render.cjs`.
Alternatively set `AXENTRA_SHARP_PATH` to an installed sharp package directory.
Copy the master to `packaging/axentra-usb.svg` after editing it.

The Windows executable, installer and shortcuts use the ICO. The native window uses
the 256 px PNG; the app header uses the 128 px PNG. Command embeds the same SVG.
