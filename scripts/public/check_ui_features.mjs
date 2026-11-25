/** Checks permanent user features in the production UI artifact.
 * Depends on ui/dist; called after the release-mode UI build.
 */
import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../..", import.meta.url));
// Optional first argument: another assets folder (tests); default is the built UI.
const dist = process.argv[2]
  ? resolve(process.argv[2])
  : resolve(root, "ui/dist/assets");
const scripts = readdirSync(dist).filter((name) => name.endsWith(".js"));
const entry = scripts.filter((name) => /^index-.*\.js$/.test(name));
if (entry.length !== 1)
  throw new Error(`expected one production entry chunk, found ${entry.length}`);
// Lazy pages (Settings since first paint) are their own chunks: a feature counts in any production chunk.
const bundle = scripts
  .map((name) => readFileSync(resolve(dist, name), "utf8"))
  .join("\n");
for (const [feature, marker] of [
  ["plan-usage", "Monthly plan usage"],
  ["context-composition", "Context breakdown"],
  ["feedback", "settings.send-feedback"],
]) {
  const present = bundle.includes(marker);
  if (!present) throw new Error(`${feature} UI must always be present`);
}
console.log(
  "Production UI: plan usage, context breakdown and feedback permanent",
);
