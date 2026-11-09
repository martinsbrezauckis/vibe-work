/** Builds the tool sidecar with the same video and feedback features as the app's default release.
 * Depends on app/Cargo.toml and Cargo; called by release builders before Tauri packages the sidecar.
 */
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const manifest = readFileSync(
  fileURLToPath(new URL("../../app/Cargo.toml", import.meta.url)),
  "utf8",
);
const defaults = manifest.match(/^default = \[([^\]]+)\]/m);
if (!defaults) throw new Error("app/Cargo.toml has no default feature list");
const selected = new Set(
  [...defaults[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]),
);
const linuxVideo = selected.has("linux-video");
const video = selected.has("video") || linuxVideo;
const feedback = selected.has("feedback");
const shipping = [
  ...(video ? ["video"] : []),
  ...(feedback ? ["feedback"] : []),
  ...(linuxVideo ? ["linux-video"] : []),
];
if (process.argv[2] === "--print") {
  console.log(
    JSON.stringify({
      feedback: feedback ? "1" : "0",
      video: video ? "1" : "0",
      linuxVideo: linuxVideo ? "1" : "0",
    }),
  );
} else {
  if (process.argv[2] !== "--" || process.argv[3] !== "cargo")
    throw new Error("usage: release_sidecar.mjs -- cargo build ... | --print");
  const args = process.argv.slice(4);
  const featuresAt = args.indexOf("--features");
  if (!args.includes("vw-agent-tools"))
    throw new Error("release_sidecar.mjs needs a vw-agent-tools build");
  if (featuresAt >= 0 && !args[featuresAt + 1])
    throw new Error("--features needs a value");
  const controlled = new Set(["video", "linux-video", "feedback"]);
  const requested = (featuresAt < 0 ? "" : args[featuresAt + 1])
    .split(",")
    .filter(Boolean)
    .filter(
      (feature) => !controlled.has(feature.replace(/^vw-agent-tools\//, "")),
    );
  const features = [
    ...requested,
    ...shipping.map((feature) => `vw-agent-tools/${feature}`),
  ].join(",");
  if (featuresAt < 0) {
    if (features) args.push("--features", features);
  } else if (features) args[featuresAt + 1] = features;
  else args.splice(featuresAt, 2);
  const result = spawnSync("cargo", args, { stdio: "inherit" });
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
}
