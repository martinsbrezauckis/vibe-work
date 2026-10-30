/**
 * First-paint check: what the window loads before the app first paints stays small.
 *
 * Role: launch speed. The built `ui/dist/index.html` names its entry script, module preloads and CSS; those load
 * before the first paint. Fails when (a) any of them contains the code editor (CodeMirror loads when a file editor
 * opens, never at startup) or (b) their total passes BUDGET bytes. Raise BUDGET only on purpose, in the same commit
 * as the feature that needs it.
 * Depends on: a finished UI build (`pnpm --filter vibe-work-ui build`), Node 20+.
 * Called by: scripts/public/check.sh after the UI build (and runnable alone).
 *
 * Exit code: 0 when clean, 1 on a violation or a missing build.
 */
import { existsSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

import { REPO_ROOT } from "./lib_files.mjs";

const BUDGET = 2_000_000;
// A class name only CodeMirror's view package carries; survives minification.
const EDITOR_MARK = "cm-scroller";

const dist = join(REPO_ROOT, "ui", "dist");
const indexFile = join(dist, "index.html");
if (!existsSync(indexFile)) {
  console.error(
    `first-paint check: ${indexFile} is missing; run the UI build first.`,
  );
  process.exit(1);
}
const html = readFileSync(indexFile, "utf8");
const files = [
  ...new Set(
    [...html.matchAll(/(?:src|href)="\/?([^"]+\.(?:js|css))"/g)].map(
      (m) => m[1],
    ),
  ),
];
let total = 0;
const problems = [];
for (const file of files) {
  const path = join(dist, file);
  total += statSync(path).size;
  if (readFileSync(path, "utf8").includes(EDITOR_MARK)) {
    problems.push(`${file} loads the code editor before the first paint`);
  }
}
console.log(
  `first-paint check: ${files.length} files, ${total} bytes (budget ${BUDGET}).`,
);
if (total > BUDGET) {
  problems.push(
    `${total} bytes load before the first paint (budget ${BUDGET})`,
  );
}
for (const problem of problems) {
  console.error(`first-paint check: ${problem}`);
}
if (problems.length) {
  process.exit(1);
}
console.log("first-paint check passed");
