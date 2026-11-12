/** Checks the shipping app's resolved features on every OS, using Cargo's actual dependency graph.
 * Called by check.sh and release builders; checks the shared app defaults on every target.
 * `--bundle` (release bundle builders only): a build with app/your-firefox must also name the Mozilla-signed
 * Firefox add-on input (`--firefox-addon <.release/inputs/firefox-addon/<version>>`), whose bytes must match its
 * SHA256SUMS; development and e2e builds keep Firefox Add-ons and need no input.
 */
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { basename, join } from "node:path";
import { REPO_ROOT } from "./lib_files.mjs";

const args = process.argv.slice(2);
let extraFeatures = "";
let selectedTarget;
let safariPackage = false;
let bundle = false;
let firefoxAddon;
while (args.length) {
  const option = args.shift();
  if (option === "--features") extraFeatures = args.shift() ?? "";
  else if (option === "--bundle") bundle = true;
  else if (option === "--firefox-addon") firefoxAddon = args.shift() ?? "";
  else if (option === "--target") selectedTarget = args.shift();
  else if (option === "--safari-package") safariPackage = true;
  else throw new Error(`Unknown release feature option: ${option}`);
}
const targets = selectedTarget
  ? [selectedTarget]
  : [
      "x86_64-unknown-linux-gnu",
      "x86_64-pc-windows-msvc",
      "aarch64-apple-darwin",
    ];
const required = new Map([
  ["app", ["macos-proxy", "your-edge", "your-firefox", "proxy-transports", "video"]],
  ["vw-core-api", ["your-edge", "your-firefox", "proxy-transports", "video"]],
  ["vw-agent-tools", ["video"]],
  ["vw-netguard", ["proxy-transports"]],
  ["vw-mistral", ["proxy-transports"]],
  ["vw-gate", ["proxy-transports"]],
  ["tauri", ["tray-icon"]],
]);
const permanent = ["plan-usage", "context-composition", "feedback", "click-marker"];
// Permanent behavior cannot acquire a Cargo off-path again, even when not selected in defaults.
for (const name of ["app", "vw-keyproxy", "vw-core-api", "vw-gate", "vw-agent-tools", "vw-browser", "vw-preview", "vw-webview", "vw-platform"]) {
  const path = name === "app" ? "app/Cargo.toml" : `crates/${name}/Cargo.toml`;
  const manifest = readFileSync(join(REPO_ROOT, path), "utf8");
  for (const feature of permanent)
    if (new RegExp(`^${feature}\\s*=`, "m").test(manifest))
      throw new Error(`${name}/${feature} must be permanent, not a feature switch`);
}
const forbidden = new Set([
  "your-safari",
  "video-test",
  "agent-vault",
]);
if (safariPackage) {
  if (!selectedTarget?.includes("apple-darwin"))
    throw new Error("Safari packaging requires a macOS target");
  forbidden.delete("your-safari");
}
const errors = [];
// The signed add-on input's digest, or the reason it is unusable (checked once, required per target below).
let firefoxSigned;
let firefoxRefusal = "no --firefox-addon input";
if (firefoxAddon) {
  try {
    const version = basename(firefoxAddon);
    const name = `vibe-work-firefox-${version}.xpi`;
    const digest = createHash("sha256").update(readFileSync(join(firefoxAddon, name))).digest("hex");
    const rows = readFileSync(join(firefoxAddon, "SHA256SUMS"), "utf8").split("\n").filter((l) => l.trim());
    const row = rows.length === 1 ? rows[0].trim().split(/\s+/) : [];
    if (row.length !== 2 || row[1].replace(/^\*/, "") !== name || row[0].toLowerCase() !== digest)
      firefoxRefusal = `${name} differs from its SHA256SUMS`;
    else firefoxSigned = digest;
  } catch (error) {
    firefoxRefusal = `unreadable input ${firefoxAddon} (${error.code ?? error.message})`;
  }
}
for (const target of targets) {
  const tree = execFileSync(
    "cargo",
    [
      "tree",
      ...(extraFeatures ? ["--features", extraFeatures] : []),
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
  for (const [name, features] of packages) {
    if (name !== "app" && !name.startsWith("vw-")) continue;
    for (const feature of features) {
      if (forbidden.has(feature))
        errors.push(`${target}: ${name}/${feature} is on`);
    }
  }
  if (
    target.includes("linux") &&
    !packages.get("vw-agent-tools")?.has("linux-video")
  )
    errors.push(`${target}: Linux video sidecar is off`);
  if (!packages.has("vw-video"))
    errors.push(`${target}: video module is missing`);
  if (!packages.get("vw-webview")?.has("macos-proxy"))
    errors.push(`${target}: preview guard is off`);
  if (bundle && packages.get("app")?.has("your-firefox") && !firefoxSigned)
    errors.push(`${target}: app/your-firefox needs the signed Firefox add-on input: ${firefoxRefusal}`);
}
if (errors.length) {
  for (const error of errors) console.error(error);
  process.exitCode = 1;
} else {
  console.log(
    `Release features: Edge, Firefox, proxies, tray and video on; plan usage, context breakdown, feedback and click markers permanent; agent vault off; Safari ${safariPackage ? "package enabled" : "off"}${firefoxSigned ? `; signed Firefox add-on ${firefoxSigned}` : ""}`,
  );
}
