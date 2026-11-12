/**
 * Rejects obsolete user and agent facing tab names in source, bundled skills and public guidance.
 * Depends on: local filesystem only. Called by: the UI lint command.
 */
import { readFileSync, readdirSync } from "node:fs";
import { dirname, extname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const obsolete = /\bpreview tab\b/gi;
const sourceExtensions = new Set([".rs", ".ts", ".tsx", ".css", ".md"]);

// These literal/path pairs describe browser authentication or wire syntax, not conversations.
const sessionMeanings = new Set([
  "features/settings/SignedInAccounts.tsx|Sign in once here. Your browser keeps the session so the agent can sign in to services on your behalf. Access beyond basic identity asks for your confirmation.",
  "features/settings/SignedInAccounts.tsx|Session status not checked",
  "features/settings/SignedInAccounts.tsx|Session present",
  "features/settings/SignedInAccounts.tsx|No saved session",
  "features/settings/SignedInAccounts.tsx|Session presence comes from cookies. A session may have expired.",
  "features/settings/catalog.ts|Sign in once in the browser, or clear a provider's saved session.",
  "features/shell/processRoutes.ts|action must be read, open, close, stop, session, output or stopSession",
  "features/transcript/transcriptRoutes.ts|choice must be once, session, always or no (or decision approve, approve_for_session, deny)",
  "state/sessionChanges.ts|session patch produced a non-object",
]);

/** Finds conversation copy without treating protocol keys, CSS classes or search keywords as labels. */
export function chatWording(path, source) {
  const parsed = ts.createSourceFile(
    path,
    source,
    ts.ScriptTarget.Latest,
    true,
    path.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
  const findings = [];
  function visit(node) {
    const isText =
      ts.isStringLiteral(node) ||
      ts.isNoSubstitutionTemplateLiteral(node) ||
      ts.isTemplateHead(node) ||
      ts.isTemplateMiddle(node) ||
      ts.isTemplateTail(node) ||
      ts.isJsxText(node);
    if (isText && /\bsessions?\b/i.test(node.text)) {
      const value = node.text.trim().replace(/\s+/g, " ");
      const parent = node.parent;
      const key =
        ts.isPropertyAssignment(parent) || ts.isJsxAttribute(parent)
          ? parent.name.getText(parsed).replace(/^['"]|['"]$/g, "")
          : "";
      const technical =
        ["className", "id", "keywords", "action", "data-testid"].includes(key) ||
        ts.isLiteralTypeNode(parent) ||
        ts.isImportDeclaration(parent) ||
        ts.isExportDeclaration(parent);
      if (
        !technical &&
        (/\s/.test(value) || /^Sessions?$/.test(value)) &&
        !sessionMeanings.has(`${path}|${value}`)
      ) {
        const line = parsed.getLineAndCharacterOfPosition(node.getStart(parsed)).line + 1;
        findings.push(`${path}:${line}: use chat for a conversation: ${JSON.stringify(value)}`);
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(parsed);
  return findings;
}

export function checkChatWording() {
  const ui = join(root, "ui/src");
  // Allowlist keys and the protocol exclusion use "/" paths; on Windows path.relative and readdir give a backslash.
  const slash = (path) => path.split(sep).join("/");
  return files(ui)
    .filter((file) => /\.tsx?$/.test(file) && !slash(file).includes("/protocol/"))
    .flatMap((file) => chatWording(slash(relative(ui, file)), readFileSync(file, "utf8")));
}

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
  const findings = [...checkTabWording(), ...checkChatWording()];
  if (findings.length > 0) {
    process.stderr.write(
      `${findings.join("\n")}\nvocabulary-drift: ${findings.length} violation(s)\n`,
    );
    process.exitCode = 1;
  } else process.stdout.write("vocabulary-drift: PASS\n");
}
