// Generate EnvBox icon set from a source PNG, following Veya's icons/ layout.
// Usage: node scripts/generate-icons.mjs <source.png> [outDir]
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const sharpPath = process.env.SHARP_PATH
  ?? 'D:\\Program Files\\Xiaomi MiMo\\resources\\runtimes\\win32-x64\\node_modules\\sharp\\lib\\index.js';
const sharp = require(sharpPath);

const src = process.argv[2];
const outDir = process.argv[3] ?? 'icons';
if (!src) {
  console.error('usage: node scripts/generate-icons.mjs <source.png> [outDir]');
  process.exit(1);
}

const SIZES = [16, 24, 32, 48, 64, 128, 256, 512];
// Veya also ships 128x128@2x (256) as a separate file name.
const EXTRA = [
  { name: '128x128@2x.png', size: 256 },
];

fs.mkdirSync(outDir, { recursive: true });

function pngBuffer(w, h) {
  return sharp(src)
    .resize(w, h, { fit: 'contain', background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .png()
    .toBuffer();
}

// PNG-in-ICO container (Vista+; matches modern Windows icon embedding).
function buildIco(entries) {
  // entries: [{ size, png: Buffer }]
  const count = entries.length;
  const headerSize = 6 + count * 16;
  let offset = headerSize;
  const dir = Buffer.alloc(headerSize);
  dir.writeUInt16LE(0, 0); // reserved
  dir.writeUInt16LE(1, 2); // type icon
  dir.writeUInt16LE(count, 4);
  for (let i = 0; i < count; i++) {
    const e = entries[i];
    const off = 6 + i * 16;
    const dim = e.size >= 256 ? 0 : e.size;
    dir.writeUInt8(dim, off + 0); // width
    dir.writeUInt8(dim, off + 1); // height
    dir.writeUInt8(0, off + 2); // colors
    dir.writeUInt8(0, off + 3); // reserved
    dir.writeUInt16LE(1, off + 4); // planes
    dir.writeUInt16LE(32, off + 6); // bpp
    dir.writeUInt32LE(e.png.length, off + 8);
    dir.writeUInt32LE(offset, off + 12);
    offset += e.png.length;
  }
  return Buffer.concat([dir, ...entries.map((e) => e.png)]);
}

const main = async () => {
  const meta = await sharp(src).metadata();
  console.log('source', meta.width, meta.height, meta.channels, 'ch');

  // Canonical source + master
  const master = await sharp(src)
    .resize(512, 512, { fit: 'contain', background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .png()
    .toBuffer();
  fs.writeFileSync(path.join(outDir, 'icon.png'), master);
  fs.writeFileSync(path.join(outDir, '512x512.png'), master);

  for (const size of SIZES) {
    if (size === 512) continue;
    const buf = await pngBuffer(size, size);
    fs.writeFileSync(path.join(outDir, `${size}x${size}.png`), buf);
  }

  for (const { name, size } of EXTRA) {
    const buf = await pngBuffer(size, size);
    fs.writeFileSync(path.join(outDir, name), buf);
  }

  // 32x32.rgba — raw top-down RGBA (Veya tray path)
  const rgba = await sharp(src)
    .resize(32, 32, { fit: 'contain', background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .ensureAlpha()
    .raw()
    .toBuffer();
  fs.writeFileSync(path.join(outDir, '32x32.rgba'), rgba);

  // ICO: standard Windows sizes + 256 PNG
  const icoSizes = [16, 24, 32, 48, 64, 128, 256];
  const entries = [];
  for (const size of icoSizes) {
    const png = await pngBuffer(size, size);
    entries.push({ size, png });
  }
  fs.writeFileSync(path.join(outDir, 'icon.ico'), buildIco(entries));

  const files = fs.readdirSync(outDir).sort();
  for (const f of files) {
    const st = fs.statSync(path.join(outDir, f));
    console.log(`${f}\t${st.size}`);
  }
};

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
