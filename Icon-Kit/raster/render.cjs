// Preserve Archive's installed ICO; extract native frames and derive larger previews.
const fs = require('fs');
const path = require('path');
const sharp = process.env.AXENTRA_SHARP_PATH ? require(process.env.AXENTRA_SHARP_PATH)
  : require(require.resolve('sharp', { paths: [path.join(__dirname, '../svg')] }));
const sizes = [16, 24, 32, 48, 64, 128, 256, 512];
const master = fs.readFileSync(path.join(__dirname, 'archive.ico'));

function frame(size) {
  if (master.readUInt16LE(0) !== 0 || master.readUInt16LE(2) !== 1) throw new Error('Invalid Archive ICO');
  for (let i = 0; i < master.readUInt16LE(4); i++) {
    const p = 6 + i * 16, n = master[p] || 256;
    if (n !== size || (master[p + 1] || 256) !== size) continue;
    const length = master.readUInt32LE(p + 8), offset = master.readUInt32LE(p + 12);
    if (offset + length > master.length) throw new Error('ICO frame outside file');
    const bytes = master.subarray(offset, offset + length);
    if (bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return sharp(bytes);
    if (bytes.readUInt32LE(0) !== 40 || bytes.readInt32LE(4) !== size || bytes.readInt32LE(8) !== size * 2
        || bytes.readUInt16LE(12) !== 1 || bytes.readUInt16LE(14) !== 32 || bytes.readUInt32LE(16) !== 0) {
      throw new Error('Expected a 32-bit uncompressed ICO DIB');
    }
    const maskRow = Math.ceil(size / 32) * 4;
    if (bytes.length !== 40 + size * size * 4 + maskRow * size) throw new Error('Invalid DIB size');
    // This approved ICO has an empty AND mask and per-pixel BGRA alpha.
    if (bytes.subarray(40 + size * size * 4).some(b => b !== 0)) throw new Error('Unexpected ICO mask');
    const rgba = Buffer.alloc(size * size * 4);
    for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
      const s = 40 + ((size - y - 1) * size + x) * 4, d = (y * size + x) * 4;
      rgba[d] = bytes[s + 2]; rgba[d + 1] = bytes[s + 1]; rgba[d + 2] = bytes[s]; rgba[d + 3] = bytes[s + 3];
    }
    return sharp(rgba, { raw: { width: size, height: size, channels: 4 } });
  }
  throw new Error(`Missing native ICO frame: ${size}`);
}

function render(size) {
  return [16, 24, 32, 48, 64, 256].includes(size)
    ? frame(size).png().toBuffer()
    : frame(256).resize(size, size, { kernel: 'lanczos3' }).png().toBuffer();
}

(async () => {
  if (!process.argv[2]) throw new Error('Usage: node raster/render.cjs OUTPUT_DIR (after Rust + SVG export)');
  const out = path.resolve(process.argv[2]), preview = path.join(out, 'preview.png');
  const previous = fs.readFileSync(preview), meta = await sharp(previous).metadata();
  const source = fs.readFileSync(path.join(__dirname, '../renderer/src/suite.rs'), 'utf8');
  const apps = [...source.match(/pub const APPS[^=]*=\s*&\[([\s\S]*?)\];/)[1].matchAll(/"([a-z0-9-]+)"/g)].map(m => m[1]);
  if (apps.includes('archive') || meta.width !== apps.length * 200 + 20 || meta.height !== 452) {
    throw new Error('Expected a fresh Rust + SVG preview, before Archive is appended');
  }
  for (const size of sizes) fs.writeFileSync(path.join(out, `archive-${size}.png`), await render(size));
  fs.writeFileSync(path.join(out, 'archive.ico'), master);
  const ox = meta.width, layers = [{ input: previous, left: 0, top: 0 }];
  for (const [size, scale, top] of [[160, 1, 20], [32, 4, 200], [16, 4, 348]]) {
    let image = await render(size);
    if (scale !== 1) image = await sharp(image).resize(size * scale, size * scale, { kernel: 'nearest' }).png().toBuffer();
    layers.push({ input: image, left: ox, top });
  }
  const sheet = await sharp({ create: { width: meta.width + 200, height: 452, channels: 4, background: '#0b0e1e' } })
    .composite(layers).png().toBuffer();
  fs.writeFileSync(preview, sheet);
  console.log('Added Archive: original eight-frame ICO, eight PNGs and the ninth preview column.');
})().catch(error => { console.error(error); process.exit(1); });
