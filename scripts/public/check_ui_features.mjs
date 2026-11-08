/** Checks the production UI artifact built with release_ui_env.mjs against the app's release defaults.
 * Depends on app/Cargo.toml and ui/dist; called after the release-mode UI build.
 */
import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../..", import.meta.url));
const cargo = readFileSync(resolve(root, "app/Cargo.toml"), "utf8");
const defaults = cargo.match(/^default = \[([^\]]+)\]/m);
if (!defaults) throw new Error("app/Cargo.toml has no default feature list");
const selected = new Set(
  [...defaults[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]),
);
const dist = resolve(root, "ui/dist/assets");
const entry = readdirSync(dist).filter((name) => /^index-.*\.js$/.test(name));
if (entry.length !== 1)
  throw new Error(`expected one production entry chunk, found ${entry.length}`);
const bundle = readFileSync(resolve(dist, entry[0]), "utf8");
for (const [feature, marker] of [
  ["plan-usage", "Monthly plan usage"],
  ["context-composition", "Context breakdown"],
  ["feedback", "settings.send-feedback"],
]) {
  const present = bundle.includes(marker);
  if (present !== selected.has(feature))
    throw new Error(
      `${feature} UI is ${present ? "present" : "absent"}; app default says ${selected.has(feature) ? "on" : "off"}`,
    );
}
console.log(
  `Production UI: plan usage ${selected.has("plan-usage") ? "on" : "off"}, context breakdown ${selected.has("context-composition") ? "on" : "off"}, feedback ${selected.has("feedback") ? "on" : "off"}`,
);
