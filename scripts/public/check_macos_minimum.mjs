/**
 * macOS minimum check: the macOS app is built with the preview network guard and every place that states the
 * oldest supported macOS says the same version.
 *
 * Role: on macOS, previews and prototype pages reach the network only through Vibe Work's network guard, which uses
 * an API that exists from macOS 14 on. The guard is the app crate's default feature `macos-proxy`; the minimum is
 * set in three build files that must agree (a lower build target would let macOS start the app on a system it was
 * not built for, and an app that links the API would not load there). README's Install table names the same
 * version. Depends on: Node 20+. Called by: scripts/public/check.sh (and alone:
 * `node scripts/public/check_macos_minimum.mjs`).
 *
 * Checks:
 *   1. app/Cargo.toml: `default` lists `macos-proxy`, and `macos-proxy` turns on `vw-webview/macos-proxy`;
 *   2. crates/vw-webview/Cargo.toml: `macos-proxy` turns on `tauri/macos-proxy` (wry's proxy configuration);
 *   3. app/tauri.conf.json `bundle.macOS.minimumSystemVersion` and .cargo/config.toml `MACOSX_DEPLOYMENT_TARGET` are
 *      the same version, at least 14.0;
 *   4. README.md's Install table says "macOS <major> or later" with that major version.
 * Exit code: 0 when all hold, 1 otherwise (each failure is printed).
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { REPO_ROOT } from "./lib_files.mjs";

/** The oldest macOS with the proxy API the preview network guard uses. */
const GUARD_MINIMUM = 14;

const read = (path) => readFileSync(resolve(REPO_ROOT, path), "utf8");
const errors = [];

/** The `[features]` table of a Cargo.toml as name -> list of entries (single-line arrays only). */
function features(path) {
  const text = read(path);
  const table = text.split(/^\[features\]\s*$/m)[1]?.split(/^\[/m)[0] ?? "";
  const out = new Map();
  for (const line of table.split("\n")) {
    const m = line.match(/^\s*([\w-]+)\s*=\s*\[(.*)\]\s*$/);
    if (m) out.set(m[1], [...m[2].matchAll(/"([^"]+)"/g)].map((x) => x[1]));
  }
  return out;
}

const app = features("app/Cargo.toml");
if (!(app.get("default") ?? []).includes("macos-proxy")) {
  errors.push("app/Cargo.toml: the default features do not include macos-proxy (macOS previews without the network guard)");
}
if (!(app.get("macos-proxy") ?? []).includes("vw-webview/macos-proxy")) {
  errors.push("app/Cargo.toml: macos-proxy does not turn on vw-webview/macos-proxy");
}
if (!(features("crates/vw-webview/Cargo.toml").get("macos-proxy") ?? []).includes("tauri/macos-proxy")) {
  errors.push("crates/vw-webview/Cargo.toml: macos-proxy does not turn on tauri/macos-proxy");
}

const bundle = JSON.parse(read("app/tauri.conf.json"))?.bundle?.macOS?.minimumSystemVersion;
const target = read(".cargo/config.toml").match(/^\s*MACOSX_DEPLOYMENT_TARGET\s*=\s*"([^"]+)"/m)?.[1];
const major = Number.parseInt(String(bundle), 10);
if (typeof bundle !== "string" || !/^\d+\.\d+$/.test(bundle)) {
  errors.push(`app/tauri.conf.json: bundle.macOS.minimumSystemVersion is ${JSON.stringify(bundle)}, not a version like "14.0"`);
} else if (major < GUARD_MINIMUM) {
  errors.push(`app/tauri.conf.json: the macOS minimum is ${bundle}; the preview network guard needs ${GUARD_MINIMUM}.0 or later`);
}
if (target !== bundle) {
  errors.push(`.cargo/config.toml: MACOSX_DEPLOYMENT_TARGET is ${JSON.stringify(target)}, the bundle minimum is ${JSON.stringify(bundle)}`);
}
const readme = read("README.md").match(/^\|\s*macOS (\d+) or later\b/m)?.[1];
if (readme !== String(major)) {
  errors.push(`README.md: the Install table says macOS ${readme ?? "(no 'macOS <n> or later' row)"} or later, the bundle minimum is ${bundle}`);
}

if (errors.length > 0) {
  for (const e of errors) console.error(e);
  console.error(`macOS minimum check FAILED (${errors.length})`);
  process.exit(1);
}
console.log(`macOS minimum check: ${bundle} everywhere, preview network guard on`);
