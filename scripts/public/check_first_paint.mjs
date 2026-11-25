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

// 2_000_000 -> 2_010_000 on purpose: the accessibility focus handover (agent frame Pause/Resume, palette
// selection, paused-tools replies) passed the old budget by 656 bytes. The command palette must stay in the first paint: keys typed
// right after Ctrl+K land in its field.
// 2_010_000 -> 2_015_000 on purpose for the 0.1.15 last fixes: voice chat silent-microphone notice and wait, page-crash notice,
// knowledge-base edits.
// 2_015_000 -> 2_017_000 on purpose for the 0.1.16 fixes: one-word voice answers matched to question cards and the
// chat-switch capture notice (68 bytes over).
// 2_017_000 -> 1_650_000 after moving voice chat, covering pages and side-panel tabs into on-demand chunks
// (measured 1,561,545 bytes; requirement: <= 2,010,000). The macOS move-to-Applications update copy adds about 486 bytes.
const BUDGET = 1_650_000;
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
