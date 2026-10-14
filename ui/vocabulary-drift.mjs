/**
 * Rejects obsolete user and agent facing tab names in source, bundled skills and public guidance.
 * Depends on: local filesystem only. Called by: the UI lint command.
 */
import { readFileSync, readdirSync } from "node:fs";
import { dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const obsolete = /\bpreview tab\b/gi;
const sourceExtensions = new Set([".rs", ".ts", ".tsx", ".css", ".md"]);

function files(path) {
  return readdirSync(path, { withFileTypes: true }).flatMap((entry) => {
    const child = join(path, entry.name);
    if (entry.isDirectory()) {
      if (["generated", "node_modules", "target"].includes(entry.name)) return [];
      return files(child);
    }
    return entry.isFile() && sourceExtensions.has(extname(child)) ? [child] : [];
  });
}

export function checkTabWording(
  paths = [
    join(root, "ui/src"),
    join(root, "crates"),
    join(root, "app/src"),
    join(root, "app/plugin/vw/skills"),
    join(root, "docs/public"),
    join(root, "README.md"),
  ],
) {
  const findings = [];
  for (const path of paths) {
    const candidates = extname(path) ? [path] : files(path);
    for (const file of candidates) {
      const source = readFileSync(file, "utf8");
      for (const match of source.matchAll(obsolete)) {
        const line = source.slice(0, match.index).split("\n").length;
        findings.push(
          `${relative(root, file)}:${line}: obsolete tab name ${JSON.stringify(match[0])}; use Browser tab`,
        );
      }
    }
  }
  return findings;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const findings = checkTabWording();
  if (findings.length > 0) {
    process.stderr.write(
      `${findings.join("\n")}\nvocabulary-drift: ${findings.length} violation(s)\n`,
    );
    process.exitCode = 1;
  } else process.stdout.write("vocabulary-drift: PASS\n");
}
