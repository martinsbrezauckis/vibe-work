/**
 * Source size check: no source file may exceed 800 lines.
 *
 * Role: keeps files small and single-purpose. Depends on: lib_files.mjs, Node 20+.
 * Called by: scripts/public/check.sh (and runnable alone: `node scripts/public/check_size.mjs`).
 *
 * Scope: project files under crates/, app/, ui/src/, tools/public/, scripts/public/ and tests/ with a
 * source extension (rs, ts, tsx, js, mjs, cjs, css, sh, py).
 *
 * Grandfather list: scripts/public/size-grandfather.json maps a path to a no-regrowth ceiling
 * (a line count). A listed file may exceed 800 lines but never its ceiling. A listed file that
 * is now within the limit, or no longer exists, is an error: delete the entry so the ratchet only
 * tightens. The list is empty today; nothing in this repo is legacy.
 *
 * Exit code: 0 when clean, 1 on any violation. Always prints the five largest files.
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { listProjectFiles, REPO_ROOT } from "./lib_files.mjs";

const LIMIT = 800;
const ROOTS = ["crates/", "app/", "ui/src/", "tools/public/", "scripts/public/", "tests/"];
const EXTENSIONS = new Set(["rs", "ts", "tsx", "js", "mjs", "cjs", "css", "sh", "py"]);
const GRANDFATHER_FILE = "scripts/public/size-grandfather.json";

/** Counts lines the way `wc -l` plus one for an unterminated last line. */
function countLines(text) {
  if (text.length === 0) return 0;
  const newlines = text.split("\n").length - 1;
  return text.endsWith("\n") ? newlines : newlines + 1;
}

const grandfather = JSON.parse(readFileSync(resolve(REPO_ROOT, GRANDFATHER_FILE), "utf8"));

const sizes = listProjectFiles()
  .filter((p) => ROOTS.some((r) => p.startsWith(r)))
  .filter((p) => EXTENSIONS.has(p.split(".").pop() ?? ""))
  .map((p) => ({ path: p, lines: countLines(readFileSync(resolve(REPO_ROOT, p), "utf8")) }));

const errors = [];
for (const { path, lines } of sizes) {
  const ceiling = grandfather[path];
  if (ceiling === undefined) {
    if (lines > LIMIT) errors.push(`${path}: ${lines} lines, limit is ${LIMIT}`);
  } else if (lines > ceiling) {
    errors.push(`${path}: ${lines} lines, grandfathered ceiling is ${ceiling} (no regrowth)`);
  } else if (lines <= LIMIT) {
    errors.push(`${path}: now ${lines} lines (within ${LIMIT}); remove it from ${GRANDFATHER_FILE}`);
  }
}
for (const path of Object.keys(grandfather)) {
  if (!sizes.some((s) => s.path === path)) {
    errors.push(`${GRANDFATHER_FILE}: entry "${path}" matches no checked file; remove it`);
  }
}

console.log(`size check: ${sizes.length} source files, limit ${LIMIT} lines. Largest 5:`);
for (const { path, lines } of [...sizes].sort((a, b) => b.lines - a.lines).slice(0, 5)) {
  console.log(`  ${String(lines).padStart(5)}  ${path}`);
}
if (errors.length > 0) {
  console.error("size check FAILED:");
  for (const e of errors) console.error(`  ${e}`);
  process.exit(1);
}
console.log("size check passed");
