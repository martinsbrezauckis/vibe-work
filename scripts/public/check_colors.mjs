/**
 * Colour check: no literal colour outside the design tokens.
 *
 * Role: keeps every colour in `ui/src/styles/tokens.css`, where light and dark themes are defined
 * together, so a component can never hard-code a value that breaks one theme. Depends on:
 * lib_files.mjs, Node 20+. Called by: scripts/public/check.sh (and alone:
 * `node scripts/public/check_colors.mjs`).
 *
 * Scope: ui/src/**.css, .ts and .tsx except tokens.css and the generated protocol folders.
 * Flags: hex colours (#rgb, #rgba, #rrggbb, #rrggbbaa), colour functions (rgb, rgba, hsl, hsla,
 * hwb, lab, lch, oklab, oklch, color()) and named colours in CSS declarations. `transparent`,
 * `currentColor`, `inherit` and `var(--token)` are fine, and so is `color-mix(...)` over tokens.
 * Comments are ignored. Exit code: 0 when clean, 1 on any violation.
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { listProjectFiles, REPO_ROOT } from "./lib_files.mjs";

const TOKENS = "ui/src/styles/tokens.css";
const SKIP_PREFIX = ["ui/src/protocol/generated/", "ui/src/protocol/core/"];
const HEX = /(?<![\w&-])#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})(?![\w-])/g;
const FUNCTION = /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(/g;
const NAMED = ["white", "black", "red", "green", "blue", "yellow", "orange", "purple", "pink", "gray", "grey", "silver", "navy", "teal", "maroon", "olive", "lime", "aqua", "fuchsia", "brown", "gold", "cyan", "magenta"];
const NAMED_IN_DECLARATION = new RegExp(
  `(?:color|background(?:-color)?|border(?:-[a-z]+)?|fill|stroke|outline(?:-color)?|box-shadow|text-shadow|caret-color)\\s*:[^;{}]*\\b(?:${NAMED.join("|")})\\b`,
  "gi",
);

/** Removes /* ... *\/ and // line comments so words in comments are not flagged. */
function stripComments(text) {
  return text.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " ")).replace(/(^|[^:])\/\/[^\n]*/g, (m, p) => p + " ".repeat(m.length - p.length));
}

const files = listProjectFiles().filter(
  (p) => p.startsWith("ui/src/") && /\.(css|ts|tsx)$/.test(p) && p !== TOKENS && !SKIP_PREFIX.some((s) => p.startsWith(s)),
);

const errors = [];
for (const path of files) {
  // Custom property names and references (--orange, var(--orange)) hold colour words but are tokens.
  const text = stripComments(readFileSync(resolve(REPO_ROOT, path), "utf8")).replace(/--[\w-]+/g, (m) => " ".repeat(m.length));
  const isCss = path.endsWith(".css");
  const lineOf = (index) => text.slice(0, index).split("\n").length;
  for (const [pattern, what] of [[HEX, "hex colour"], [FUNCTION, "colour function"], ...(isCss ? [[NAMED_IN_DECLARATION, "named colour"]] : [])]) {
    for (const match of text.matchAll(pattern)) {
      errors.push(`${path}:${lineOf(match.index)}: ${what} ${JSON.stringify(match[0].trim())} (use a token from ${TOKENS})`);
    }
  }
}

console.log(`colour check: ${files.length} UI source files outside ${TOKENS}.`);
if (errors.length > 0) {
  console.error(errors.join("\n"));
  console.error(`colour check FAILED: ${errors.length} literal colour(s)`);
  process.exit(1);
}
console.log("colour check passed");
