/** Builds the tool sidecar with feedback only when the app's default release features include it.
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
const feedback = [...defaults[1].matchAll(/"([^"]+)"/g)].some(
  (match) => match[1] === "feedback",
);
if (process.argv[2] === "--print") {
  console.log(JSON.stringify({ feedback: feedback ? "1" : "0" }));
} else {
  if (process.argv[2] !== "--" || process.argv[3] !== "cargo")
    throw new Error("usage: release_sidecar.mjs -- cargo build ... | --print");
  const args = process.argv.slice(4);
  const featuresAt = args.indexOf("--features");
  if (
    !args.includes("vw-agent-tools") ||
    featuresAt < 0 ||
    !args[featuresAt + 1]
  )
    throw new Error(
      "release_sidecar.mjs needs a vw-agent-tools build with --features",
    );
  if (feedback) args[featuresAt + 1] += ",vw-agent-tools/feedback";
  const result = spawnSync("cargo", args, { stdio: "inherit" });
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
}
