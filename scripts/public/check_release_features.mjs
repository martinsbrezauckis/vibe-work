/** Checks the shipping app's resolved features on every OS, using Cargo's actual dependency graph.
 * Called by check.sh and release builders; keeps later-release modules out of production defaults.
 */
import { execFileSync } from "node:child_process";
import { REPO_ROOT } from "./lib_files.mjs";

const targets = ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc", "aarch64-apple-darwin"];
const required = new Map([
  ["app", ["macos-proxy", "your-edge", "proxy-transports"]],
  ["vw-core-api", ["your-edge", "proxy-transports"]],
  ["vw-netguard", ["proxy-transports"]],
  ["vw-mistral", ["proxy-transports"]],
  ["vw-gate", ["proxy-transports"]],
  ["tauri", ["tray-icon"]],
]);
const forbidden = new Set(["your-firefox", "video", "video-test", "feedback", "click-marker"]);
const errors = [];
for (const target of targets) {
  const tree = execFileSync("cargo", ["tree", "--color", "never", "--locked", "-p", "app", "--target", target,
    "--edges", "normal,build", "--prefix", "none", "--format", "{p}|{f}"],
  { cwd: REPO_ROOT, encoding: "utf8", maxBuffer: 8 * 1024 * 1024 });
  const packages = new Map();
  for (const line of tree.split("\n")) {
    const [identity, enabled = ""] = line.split("|");
    packages.set(identity.split(" ")[0], new Set(enabled.replace(/ \(\*\)$/, "").split(",")));
  }
  for (const [name, features] of required) {
    for (const feature of features) {
      if (!packages.get(name)?.has(feature)) errors.push(`${target}: ${name}/${feature} is off`);
    }
  }
  for (const [name, features] of packages) {
    if (name !== "app" && !name.startsWith("vw-")) continue;
    if (name === "vw-video") errors.push(`${target}: video module is linked`);
    for (const feature of features) {
      if (forbidden.has(feature)) errors.push(`${target}: ${name}/${feature} is on`);
    }
  }
  if (!packages.get("vw-webview")?.has("macos-proxy")) errors.push(`${target}: preview guard is off`);
}
if (errors.length) {
  for (const error of errors) console.error(error);
  process.exitCode = 1;
} else {
  console.log("Release features: Edge, proxies and tray on; Firefox, video, feedback and click markers off (all OSes)");
}
