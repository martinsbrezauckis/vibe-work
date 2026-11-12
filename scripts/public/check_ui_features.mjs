/** Checks permanent user features in the production UI artifact.
 * Depends on ui/dist; called after the release-mode UI build.
 */
import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../..", import.meta.url));
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
  if (!present) throw new Error(`${feature} UI must always be present`);
}
console.log("Production UI: plan usage, context breakdown and feedback permanent");
