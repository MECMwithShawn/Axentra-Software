// The same SVG, optical-size rules and PNG/ICO encoding used by the app generators.
const fs = require('fs');
const path = require('path');
const sharp = process.env.AXENTRA_SHARP_PATH ? require(process.env.AXENTRA_SHARP_PATH) : require('sharp');
const sizes = [16, 24, 32, 48, 64, 128, 256, 512];

function sourceFor(app, size) {
  let source = fs.readFileSync(path.join(__dirname, `${app}.svg`), 'utf8');
  if (size <= 24) {
    source = app === 'horizon'
      ? source.replace('stroke-width="1.65"', 'stroke-width="2.2"')
      : source.replace('stroke-width="1.3"', `stroke-width="${size === 16 ? 2.2 : 1.7}"`).replace('stroke-width="1.4"', 'stroke-width="2"');
  }
  return source;
}

async function render(app, size) {
  return sharp(Buffer.from(sourceFor(app, size)), { density: 384 }).resize(size, size).png().toBuffer();
}

function dib(raw, n) {
  const maskRow = Math.ceil(n / 32) * 4;
  const out = Buffer.alloc(40 + n * n * 4 + maskRow * n);
  out.writeUInt32LE(40, 0); out.writeInt32LE(n, 4); out.writeInt32LE(n * 2, 8);
  out.writeUInt16LE(1, 12); out.writeUInt16LE(32, 14); out.writeUInt32LE(n * n * 4 + maskRow * n, 20);
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
    const src = (y * n + x) * 4, dst = 40 + ((n - 1 - y) * n + x) * 4;
    out[dst] = raw[src + 2]; out[dst + 1] = raw[src + 1]; out[dst + 2] = raw[src]; out[dst + 3] = raw[src + 3];
  }
  return out;
}

async function exportApp(app, out) {
  const entries = [];
  for (const n of sizes) {
    const png = await render(app, n);
    fs.writeFileSync(path.join(out, `${app}-${n}.png`), png);
    if (n <= 256) {
      const raw = await sharp(png).ensureAlpha().raw().toBuffer();
      entries.push({ n, bytes: n === 256 ? png : dib(raw, n) });
    }
  }
  const header = Buffer.alloc(6 + entries.length * 16);
  header.writeUInt16LE(1, 2); header.writeUInt16LE(entries.length, 4);
  let offset = header.length;
  entries.forEach(({ n, bytes }, i) => {
    const p = 6 + i * 16;
    header[p] = header[p + 1] = n === 256 ? 0 : n;
    header.writeUInt16LE(1, p + 4); header.writeUInt16LE(32, p + 6);
    header.writeUInt32LE(bytes.length, p + 8); header.writeUInt32LE(offset, p + 12);
    offset += bytes.length;
  });
  fs.writeFileSync(path.join(out, `${app}.ico`), Buffer.concat([header, ...entries.map(e => e.bytes)]));
}

async function updatePreview(out) {
  // Retain all Rust-rendered columns, including new apps appended to APPS.
  const file = path.join(out, 'preview.png');
  const { data, info } = await sharp(file).ensureAlpha().raw().toBuffer({ resolveWithObject: true });
  const appsSource = fs.readFileSync(path.join(__dirname, '../renderer/src/suite.rs'), 'utf8');
  const appList = appsSource.match(/pub const APPS[^=]*=\s*&\[([\s\S]*?)\];/);
  if (!appList) throw new Error('Cannot read renderer app order');
  const apps = [...appList[1].matchAll(/"([a-z0-9-]+)"/g)].map(m => m[1]);
  if (info.width !== apps.length * 200 + 20 || info.height !== 452) throw new Error('Unexpected contact sheet dimensions');
  for (const app of ['crystalwire', 'horizon']) {
    const index = apps.indexOf(app);
    if (index < 0) throw new Error(`Missing app: ${app}`);
    const ox = 20 + index * 200;
    for (let y = 0; y < info.height; y++) for (let x = ox; x < ox + 200; x++) {
      const d = (y * info.width + x) * 4;
      data.set([11, 14, 30, 255], d);
    }
    for (const [n, scale, oy] of [[160, 1, 20], [32, 4, 200], [16, 4, 348]]) {
      const raw = await sharp(await render(app, n)).ensureAlpha().raw().toBuffer();
      for (let y = 0; y < n * scale; y++) for (let x = 0; x < n * scale; x++) {
        const s = (Math.floor(y / scale) * n + Math.floor(x / scale)) * 4;
        const d = ((oy + y) * info.width + ox + x) * 4;
        const a = raw[s + 3] / 255;
        for (let k = 0; k < 3; k++) data[d + k] = Math.round(raw[s + k] * a + data[d + k] * (1 - a));
      }
    }
  }
  fs.writeFileSync(file, await sharp(data, { raw: info }).png().toBuffer());
}

(async () => {
  if (process.argv[2] === '--versions') {
    console.log(JSON.stringify({ node: process.version, platform: process.platform, arch: process.arch, ...sharp.versions }, null, 2));
    return;
  }
  if (!process.argv[2]) throw new Error('Usage: node svg/render.cjs OUTPUT_DIR (after the Rust export)');
  const out = path.resolve(process.argv[2]);
  if (!fs.existsSync(path.join(out, 'preview.png'))) throw new Error('Run scripts/export.py to create the complete export first');
  for (const app of ['horizon', 'crystalwire']) await exportApp(app, out);
  await updatePreview(out);
  console.log('Updated Horizon and CrystalWire: 16 PNGs, two seven-frame ICOs and the contact sheet.');
})().catch(error => { console.error(error); process.exit(1); });
