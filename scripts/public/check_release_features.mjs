/** Checks the shipping app's resolved features on every OS, using Cargo's actual dependency graph.
 * Called by check.sh and release builders; checks the shared app defaults on every target.
 */
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { REPO_ROOT } from "./lib_files.mjs";

const targets = [
  "x86_64-unknown-linux-gnu",
  "x86_64-pc-windows-msvc",
  "aarch64-apple-darwin",
];
const manifest = readFileSync(join(REPO_ROOT, "app/Cargo.toml"), "utf8");
const defaults = manifest.match(/^default = \[([^\]]+)\]/m);
if (!defaults) throw new Error("app/Cargo.toml has no default feature list");
const selected = new Set(
  [...defaults[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]),
);
const required = new Map([
  ["app", ["macos-proxy", "your-edge", "proxy-transports", "video"]],
  ["vw-core-api", ["your-edge", "proxy-transports", "video"]],
  ["vw-agent-tools", ["video"]],
  ["vw-netguard", ["proxy-transports"]],
  ["vw-mistral", ["proxy-transports"]],
  ["vw-gate", ["proxy-transports"]],
  ["tauri", ["tray-icon"]],
]);
const controlled = new Map([
  ["plan-usage", ["app", "vw-keyproxy"]],
  ["context-composition", ["app", "vw-core-api", "vw-gate"]],
  ["feedback", ["app", "vw-core-api", "vw-agent-tools"]],
  [
    "click-marker",
    [
      "app",
      "vw-core-api",
      "vw-browser",
      "vw-preview",
      "vw-webview",
      "vw-platform",
    ],
  ],
]);
const forbidden = new Set([
  "your-firefox",
  "your-safari",
  "video-test",
  "agent-vault",
]);
const errors = [];
for (const target of targets) {
  const tree = execFileSync(
    "cargo",
    [
      "tree",
      "--color",
      "never",
      "--locked",
      "-p",
      "app",
      "--target",
      target,
      "--edges",
      "normal,build",
      "--prefix",
      "none",
      "--format",
      "{p}|{f}",
    ],
    { cwd: REPO_ROOT, encoding: "utf8", maxBuffer: 8 * 1024 * 1024 },
  );
  const packages = new Map();
  for (const line of tree.split("\n")) {
    const [identity, enabled = ""] = line.split("|");
    packages.set(
      identity.split(" ")[0],
      new Set(enabled.replace(/ \(\*\)$/, "").split(",")),
    );
  }
  for (const [name, features] of required) {
    for (const feature of features) {
      if (!packages.get(name)?.has(feature))
        errors.push(`${target}: ${name}/${feature} is off`);
    }
  }
  for (const [feature, names] of controlled) {
    for (const name of names) {
      const present = packages.get(name)?.has(feature) ?? false;
      if (present !== selected.has(feature))
        errors.push(
          `${target}: ${name}/${feature} is ${present ? "on" : "off"}, app default says ${selected.has(feature) ? "on" : "off"}`,
        );
    }
  }
  for (const [name, features] of packages) {
    if (name !== "app" && !name.startsWith("vw-")) continue;
    for (const feature of features) {
      if (forbidden.has(feature))
        errors.push(`${target}: ${name}/${feature} is on`);
    }
  }
  if (!packages.has("vw-video"))
    errors.push(`${target}: video module is missing`);
  if (!packages.get("vw-webview")?.has("macos-proxy"))
    errors.push(`${target}: preview guard is off`);
}
if (errors.length) {
  for (const error of errors) console.error(error);
  process.exitCode = 1;
} else {
  console.log(
    `Release features: Edge, proxies, tray and video on; plan usage, context breakdown and click markers follow app defaults; feedback ${selected.has("feedback") ? "on" : "off"}; Firefox, Safari and agent vault off (all OSes)`,
  );
}
