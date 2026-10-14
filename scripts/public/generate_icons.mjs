/**
 * Regenerates the desktop icon set from app/icons/source/icon.svg, checking it against the UI mark.
 * Depends on Node and the pinned Tauri CLI; called by `pnpm icons:generate` after `pnpm install`.
 * Fits every frame to integer cells and a 1 px edge, retains desktop outputs and removes scratch.
 */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const icons = join(root, "app/icons");
const source = join(icons, "source/icon.svg");
const cli = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");
const svg = readFileSync(source, "utf8");
const brand = readFileSync(join(root, "ui/src/components/BrandMark.tsx"), "utf8");
const shell = readFileSync(join(root, "ui/src/styles/shell.css"), "utf8");
const tokens = readFileSync(join(root, "ui/src/styles/tokens.css"), "utf8");

// Fail on drift instead of quietly shipping a different mark from the one inside the app.
const uiCells = [...brand.matchAll(/\[(\d+), (\d+), "(c[123])"\]/g)].map(([, x, y, tone]) => [
  Number(x),
  Number(y),
  tone,
]);
const iconCells = [
  ...svg.matchAll(
    /<rect class="(c[123])" x="(\d+)" y="(\d+)" width="2" height="2" fill="(#[\da-f]{6})"/g,
  ),
];
assert.equal(uiCells.length, 13, "Expected the 13-cell BrandMark");
assert.deepEqual(
  iconCells.map(([, tone, x, y]) => [Number(x), Number(y), tone]),
  uiCells,
);
for (const [, tone, , , fill] of iconCells) {
  const token = shell.match(
    new RegExp(`\\.brand-mark \\.${tone}\\s*\\{\\s*fill: var\\((--[\\w-]+)\\)`),
  )?.[1];
  assert.ok(token, `Missing CSS tone ${tone}`);
  assert.equal(fill, tokens.match(new RegExp(`${token}: (#[\\da-f]{6});`))?.[1]);
}
const viewport = svg.match(/viewBox="0 0 (\d+) (\d+)"/);
const mark = svg.match(/<g id="mark" transform="translate\((\d+) (\d+)\) scale\((\d+)\)"/);
const tile = svg.match(/<rect id="tile"[^>]+\/>/)?.[0];
assert.ok(viewport && mark && tile, "Expected a square viewport, scaled mark and rounded tile");
const masterSize = Number(viewport[1]);
const masterScale = Number(mark[3]);
assert.equal(Number(viewport[2]), masterSize);
assert.equal(Number(mark[1]), (masterSize - 14 * masterScale) / 2);
assert.equal(Number(mark[2]), (masterSize - 10 * masterScale) / 2);
const tileAttrs = Object.fromEntries(
  [...tile.matchAll(/([\w-]+)="([^"]+)"/g)].map((m) => m.slice(1)),
);
const masterEdge = Number(tileAttrs["stroke-width"]);
assert.equal(masterEdge, 1, "Expected the master tile's 1 px edge");
assert.equal(tileAttrs.x, tileAttrs.y);
assert.equal(tileAttrs.width, tileAttrs.height);

const scratchRoot = join(root, ".scratch");
mkdirSync(scratchRoot, { recursive: true });
const scratch = mkdtempSync(join(scratchRoot, "icons-"));
const generated = join(scratch, "generated");

/** Runs the same pinned CLI as `pnpm tauri icon`, serially with at most two rendering threads. */
function render(input) {
  const args = ["icon", input, "--output", generated];
  execFileSync(process.execPath, [cli, ...args], {
    cwd: root,
    env: { ...process.env, RAYON_NUM_THREADS: "2" },
    stdio: ["ignore", "pipe", "pipe"],
  });
}

/** Derives one pixel-fitted SVG from the master; keeps the tile edge inside its original outline. */
function frameSvg(size) {
  const ratio = size / masterSize;
  const scale = Math.max(1, Math.floor(masterScale * ratio));
  const edge = size >= 32 ? 1 : 0;
  const inset = (Number(tileAttrs.x) - masterEdge / 2) * ratio;
  const width = (Number(tileAttrs.width) + masterEdge) * ratio - edge;
  const geometry = {
    x: inset + edge / 2,
    y: inset + edge / 2,
    width,
    height: width,
    rx: (Number(tileAttrs.rx) + masterEdge / 2) * ratio - edge / 2,
    "stroke-width": edge,
  };
  const fittedTile = tile.replace(
    /\b(x|y|width|height|rx|stroke-width)="[^"]+"/g,
    (_, attr) => `${attr}="${geometry[attr]}"`,
  );
  return svg
    .replace(
      /width="1024" height="1024" viewBox="[^"]+"/,
      `width="${size}" height="${size}" viewBox="0 0 ${size} ${size}"`,
    )
    .replace(tile, fittedTile)
    .replace(
      mark[0],
      `<g id="mark" transform="translate(${(size - 14 * scale) / 2} ${(size - 10 * scale) / 2}) scale(${scale})"`,
    );
}

/** Reads Tauri's six PNG frames, retaining the directory for pixel-fitted replacements. */
function icoFrames(ico) {
  assert.equal(ico.readUInt16LE(0), 0);
  assert.equal(ico.readUInt16LE(2), 1);
  assert.equal(ico.readUInt16LE(4), 6);
  const frames = new Map();
  for (let i = 0; i < 6; i++) {
    const entry = 6 + i * 16;
    const size = ico[entry] || 256;
    assert.equal(size, [16, 24, 32, 48, 64, 256][i]);
    assert.equal(ico[entry + 1] || 256, size);
    const start = ico.readUInt32LE(entry + 12);
    const png = ico.subarray(start, start + ico.readUInt32LE(entry + 8));
    assert.equal(png.toString("ascii", 1, 4), "PNG");
    frames.set(size, png);
  }
  return frames;
}

/** Replaces all ICO frames with the matching per-size Tauri renders. */
function fittedIco(ico, frames) {
  const directory = Buffer.from(ico.subarray(0, 6 + 6 * 16));
  const images = [];
  let offset = directory.length;
  for (let i = 0; i < 6; i++) {
    const entry = 6 + i * 16;
    const size = ico[entry] || 256;
    const image = frames.get(size);
    assert.ok(image, `Missing ICO frame ${size}`);
    directory.writeUInt32LE(image.length, entry + 8);
    directory.writeUInt32LE(offset, entry + 12);
    images.push(image);
    offset += image.length;
  }
  return Buffer.concat([directory, ...images]);
}

/** Reads ICNS chunks and their pixel sizes, including the legacy 16/32 px RGB and mask pairs. */
function icnsFrames(icns) {
  assert.equal(icns.toString("ascii", 0, 4), "icns");
  assert.equal(icns.readUInt32BE(4), icns.length);
  const entries = [];
  for (let offset = 8; offset < icns.length;) {
    const length = icns.readUInt32BE(offset + 4);
    assert.ok(length >= 8 && offset + length <= icns.length);
    const chunk = icns.subarray(offset, offset + length);
    const type = chunk.toString("ascii", 0, 4);
    const legacy = { is32: 16, s8mk: 16, il32: 32, l8mk: 32 }[type];
    if (!legacy) assert.equal(chunk.toString("ascii", 9, 12), "PNG", `Unknown ICNS chunk ${type}`);
    const size = legacy ?? chunk.readUInt32BE(24);
    entries.push({ type, size, chunk });
    offset += length;
  }
  return entries;
}

try {
  const frames = new Map();
  const chunks = new Map();
  let ico;
  let expectedTypes;
  // WORKAROUND: Tauri proportionally scales one SVG for every frame, losing integer grid units
  // and a constant-width edge. Assemble its per-size renders until it supports per-frame SVGs.
  for (const size of [16, 24, 32, 48, 64, 128, 256, 512, 1024]) {
    const input = join(scratch, `${size}.svg`);
    writeFileSync(input, frameSvg(size));
    render(input);
    ico = readFileSync(join(generated, "icon.ico"));
    const icoPngs = icoFrames(ico);
    const icns = icnsFrames(readFileSync(join(generated, "icon.icns")));
    const types = icns.map(({ type }) => type).sort();
    expectedTypes ??= types;
    assert.deepEqual(types, expectedTypes);
    for (const { type, size: pixels, chunk } of icns) {
      if (pixels === size) chunks.set(type, chunk);
    }
    const png =
      icoPngs.get(size) ??
      icns
        .find(
          ({ size: pixels, chunk }) => pixels === size && chunk.toString("ascii", 9, 12) === "PNG",
        )
        ?.chunk.subarray(8);
    assert.ok(png, `Missing ${size} px render`);
    assert.equal(png.readUInt32BE(16), size);
    assert.equal(png.readUInt32BE(20), size);
    frames.set(size, png);
  }
  assert.deepEqual([...chunks.keys()].sort(), expectedTypes);
  const entries = [...chunks.values()].sort((a, b) =>
    Buffer.compare(a.subarray(0, 4), b.subarray(0, 4)),
  );
  const header = Buffer.alloc(8);
  header.write("icns");
  header.writeUInt32BE(8 + entries.reduce((sum, chunk) => sum + chunk.length, 0), 4);
  const outputs = [
    ["icon.ico", fittedIco(ico, frames)],
    ["icon.icns", Buffer.concat([header, ...entries])],
    ["icon.png", frames.get(512)],
    ["128x128@2x.png", frames.get(256)],
    ["app-icon.png", frames.get(1024)],
    ...[16, 24, 32, 48, 64, 128].map((size) => [`${size}x${size}.png`, frames.get(size)]),
  ];
  // Read and validate every expected output before replacing any checked-in asset.
  for (const [file, bytes] of outputs) writeFileSync(join(icons, file), bytes);
  console.log(
    "Desktop icons regenerated (ICO: 16, 24, 32, 48, 64, 256 px; ICNS; PNGs). Scratch removed on exit.",
  );
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
