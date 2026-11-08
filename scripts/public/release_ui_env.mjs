/** Derives release UI flags from app's default Cargo features before a UI or Tauri build.
 * Depends on app/Cargo.toml; called by every release builder so each feature has one switch.
 */
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const manifest = readFileSync(
  fileURLToPath(new URL("../../app/Cargo.toml", import.meta.url)),
  "utf8",
);
const features = manifest.match(/^default = \[([^\]]+)\]/m);
if (!features) throw new Error("app/Cargo.toml has no default feature list");
const selected = new Set(
  [...features[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]),
);
const flags = {
  VITE_PLAN_USAGE: selected.has("plan-usage") ? "1" : "0",
  VITE_CONTEXT_COMPOSITION: selected.has("context-composition") ? "1" : "0",
  VITE_FEEDBACK: selected.has("feedback") ? "1" : "0",
};
if (process.argv[2] === "--print") {
  console.log(JSON.stringify(flags));
} else if (process.argv[2] === "--shell") {
  for (const [name, value] of Object.entries(flags))
    console.log(`export ${name}=${value}`);
} else {
  if (process.argv[2] !== "--" || !process.argv[3])
    throw new Error(
      "usage: release_ui_env.mjs -- command [args...] | --print | --shell",
    );
  if (process.platform === "win32")
    throw new Error(
      "use --print to set release UI flags in Windows PowerShell",
    );
  const result = spawnSync(process.argv[3], process.argv.slice(4), {
    stdio: "inherit",
    env: { ...process.env, ...flags },
  });
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
}
