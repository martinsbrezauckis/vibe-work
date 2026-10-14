/**
 * Public-paths scan: guards the positive allowlist in release/public-paths.json.
 *
 * Role: stops private material and forbidden strings from reaching the public export, and from
 * being committed straight into the public repository.
 * Depends on: lib_files.mjs, Node 20+. Called by: scripts/public/check.sh (or run alone).
 * Usage: node check_public_paths.mjs [--public]   (VW_PUBLIC_REPO=1 is the same as --public)
 *
 * Rules 1 to 3 apply to project files (tracked plus untracked-not-ignored), in both modes:
 *   1. Marker scan: a file inside the allowlist (a `roots` prefix or an exact `files` entry) must
 *      not contain any `forbiddenPublicMarkers` string. The allowlist file itself is exempt
 *      because it has to spell the markers out.
 *   2. Boundary: a file inside the allowlist must not also match `neverExport`. A pattern matches
 *      by path prefix; patterns ending in "private/" also match at any directory depth
 *      (for example tests/public/fixtures/private/).
 *   3. Existence: every exact `files` entry and every `roots` entry names something that
 *      exists in the project, so the allowlist never lists a file the export would silently skip.
 *
 * Rule 4 applies in public-repository mode only:
 *   4. Closed set: every path the repository TRACKS (`git ls-files`) must be inside the allowlist. A tracked
 *      file outside it (a stray `private/` note, an extra root file) fails and is listed. The exporter only ever
 *      copies allowlisted paths, so such a file can only arrive through a direct commit or a pull request,
 *      where rules 1 and 2 never look at it (they skip files outside the allowlist).
 *
 * Which mode runs:
 *   - Public-repository mode when `--public` is given or VW_PUBLIC_REPO=1 is set. CI sets the variable, so a
 *     change cannot switch the mode off through repository content alone.
 *   - Otherwise the mode is detected. The checkout is the private SOURCE tree only when git tracks files under
 *     BOTH source-only folders (SOURCE_ONLY_DIRS below: the project notes folder and the private docs folder). There
 *     tracked files outside the allowlist are legitimate, only rules 1 to 3 run. Any other checkout (the public
 *     export, a public clone) runs in public-repository mode.
 *   - Detection looks at tracked files, not at folders on disk, so a maintainer's untracked notes folder in a
 *     public clone does not switch the guard off. It requires both folders, so a single stray file in one of
 *     them cannot make the public repository pass as the source tree. A commit that deliberately adds tracked
 *     files under both would still be taken for the source tree, which is why CI sets the variable.
 *   - When unsure the guard fails loudly: a source tree missing one folder is reported as a public repository
 *     with a long offender list, never the reverse.
 *
 * Matching details:
 *   - A marker that starts and ends with a letter or digit (a short host nickname, say) only
 *     matches as a whole token, not inside a longer alphanumeric run. Without this, a three
 *     character marker matched inside the base64 integrity hashes of pnpm-lock.yaml.
 *   - In lockfiles, lines carrying registry hashes ("integrity:" / "checksum =") are not scanned:
 *     they are base64 noise, and every other lockfile line (paths, URLs, names) still is.
 *   - Binary files are scanned as latin1 text (a marker inside a binary is still a leak).
 * Extra markers: when `release/private/forbidden-markers.json` exists (maintainers' checkouts; it is never
 * exported), its `forbiddenPublicMarkers` are checked too. They name local paths, hosts and machines, which the
 * public allowlist file must not spell out itself. A public checkout has no such file and checks the public list.
 * Exit code: 0 when clean, 1 on any violation (each one printed with path and reason), 2 on a bad argument.
 */
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

import { listProjectFiles, listTrackedFiles, REPO_ROOT } from "./lib_files.mjs";

// Folders that exist only in the private source tree. Spelled without a trailing slash on purpose: the
// public allowlist forbids the slash forms as markers in every exported file, this script included.
const SOURCE_ONLY_DIRS = ["docs/private", ".project"];

for (const arg of process.argv.slice(2)) {
  if (arg !== "--public") {
    console.error(`public-paths check: unknown argument "${arg}" (usage: check_public_paths.mjs [--public])`);
    process.exit(2);
  }
}
const forcedPublic =
  process.argv.includes("--public") || ["1", "true"].includes((process.env.VW_PUBLIC_REPO ?? "").toLowerCase());

const CONFIG_PATH = "release/public-paths.json";
// VW_PUBLIC_PATHS_CONFIG (absolute path) swaps in another policy file; only the private tests use it,
// to prove that a policy exporting a private folder is refused.
const configFile = process.env.VW_PUBLIC_PATHS_CONFIG ?? resolve(REPO_ROOT, CONFIG_PATH);
const config = JSON.parse(readFileSync(configFile, "utf8"));
const EXTRA_MARKERS_PATH = resolve(REPO_ROOT, "release/private/forbidden-markers.json");

for (const key of ["roots", "files", "neverExport", "forbiddenPublicMarkers"]) {
  if (!Array.isArray(config[key])) {
    console.error(`public-paths check FAILED: ${CONFIG_PATH} has no array "${key}"`);
    process.exit(1);
  }
}

/** True when `path` is covered by the positive allowlist. */
function isAllowlisted(path) {
  return (
    config.files.includes(path) ||
    config.roots.some((r) => (r.endsWith("/") ? path.startsWith(r) : path === r))
  );
}

const LOCKFILES = new Set(["pnpm-lock.yaml", "Cargo.lock"]);
const HASH_LINE = /integrity:|checksum\s*=/;

/** Builds the matcher for one marker: whole-token for alphanumeric edges, plain substring otherwise. */
function markerPattern(marker) {
  const escaped = marker.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const alnum = /[A-Za-z0-9]/;
  const left = alnum.test(marker[0] ?? "") ? "(?<![A-Za-z0-9])" : "";
  const right = alnum.test(marker[marker.length - 1] ?? "") ? "(?![A-Za-z0-9])" : "";
  return new RegExp(`${left}${escaped}${right}`);
}

/** The maintainers' extra markers (see the header), or none when the file is absent. */
function extraMarkers() {
  if (!existsSync(EXTRA_MARKERS_PATH)) return [];
  const extra = JSON.parse(readFileSync(EXTRA_MARKERS_PATH, "utf8")).forbiddenPublicMarkers;
  if (!Array.isArray(extra)) {
    console.error("public-paths check FAILED: release/private/forbidden-markers.json has no array \"forbiddenPublicMarkers\"");
    process.exit(1);
  }
  return extra;
}

const allMarkers = [...config.forbiddenPublicMarkers, ...extraMarkers()];
const markers = allMarkers.map((m) => ({ text: m, re: markerPattern(m) }));

/** True when `path` matches a neverExport pattern (see the header for the matching rule). */
function matchesNeverExport(path) {
  return config.neverExport.some(
    (pattern) =>
      path.startsWith(pattern) || (pattern.endsWith("private/") && path.includes(`/${pattern}`)),
  );
}

const errors = [];
let scanned = 0;
const trackedFiles = listTrackedFiles();
const looksLikeSourceTree = SOURCE_ONLY_DIRS.every((dir) =>
  trackedFiles.some((path) => path.startsWith(`${dir}/`)),
);
const publicRepositoryMode = forcedPublic || !looksLikeSourceTree;
if (publicRepositoryMode) {
  // Rule 4. A tracked file under neverExport that is also allowlisted is reported by rule 2 below.
  for (const path of trackedFiles) {
    if (isAllowlisted(path)) continue;
    const never = matchesNeverExport(path) ? " (also matches neverExport)" : "";
    errors.push(`${path}: tracked in the public repository but outside the public allowlist${never}`);
  }
}
const projectFiles = listProjectFiles();
for (const entry of config.files) {
  if (!projectFiles.includes(entry)) errors.push(`${entry}: listed in "files" but not in the project`);
}
for (const root of config.roots) {
  const present = root.endsWith("/")
    ? projectFiles.some((path) => path.startsWith(root))
    : projectFiles.includes(root);
  if (!present) errors.push(`${root}: listed in "roots" but nothing in the project is under it`);
}
for (const path of projectFiles) {
  if (!isAllowlisted(path)) continue;
  if (matchesNeverExport(path)) {
    errors.push(`${path}: inside the public allowlist but matches neverExport`);
    continue;
  }
  if (path === CONFIG_PATH) continue;
  scanned += 1;
  let text = readFileSync(resolve(REPO_ROOT, path)).toString("latin1");
  if (LOCKFILES.has(path)) {
    text = text
      .split("\n")
      .filter((line) => !HASH_LINE.test(line))
      .join("\n");
  }
  for (const { text: marker, re } of markers) {
    if (re.test(text)) errors.push(`${path}: contains forbidden marker "${marker}"`);
  }
}

const modeNote = publicRepositoryMode
  ? `public repository mode, ${trackedFiles.length} tracked files must be allowlisted`
  : "source tree mode";
console.log(`public-paths check (${modeNote}): ${scanned} allowlisted files scanned for ${markers.length} markers`);
if (errors.length > 0) {
  console.error("public-paths check FAILED:");
  for (const e of errors) console.error(`  ${e}`);
  process.exit(1);
}
console.log("public-paths check passed");
