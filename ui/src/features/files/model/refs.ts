/**
 * Finds file paths in chat text. Pure functions, no I/O: a candidate is only a GUESS; the core decides
 * whether it names a file the app may show (`files resolve`), and only then is it drawn as a link.
 *
 * Rules:
 * - Plain text is split on whitespace: a path with spaces is found only inside inline code or as a link target,
 *   never by gluing words together.
 * - A candidate is an absolute path (`/x/y`, `C:\x`, `C:/x`), a home path (`~/x`), a relative path that starts with
 *   `./` or `../`, a relative path with a `/` whose last part has an extension or a well-known name, or a bare file
 *   name with a known extension or name. `file://` URLs count; web URLs, `host:port` and e-mail addresses never do.
 * - A `:12`, `:12:5` or `#L12` suffix is the line (and column); trailing punctuation is not part of the path.
 * - The Markdown text itself is never changed (the transcript renders the parsed tree; see `remarkFileRefs.ts`).
 *
 * Depends on: nothing. Called by: remarkFileRefs, the tool-output linker, tests (`files-refs.test.ts`).
 */

/** One path found in a text. */
export interface PathRef {
  /** Offset of the first character of the reference (including a line suffix). */
  start: number;
  /** Offset after the last character. */
  end: number;
  /** The path as written (no line suffix, no surrounding punctuation). */
  path: string;
  /** Line number from a `:12` / `#` suffix. */
  line: number | null;
}

/** File extensions a bare name may have to count as a file (a name with a folder part may have any). */
const EXTENSIONS = new Set(
  (
    "md markdown mdx txt rst adoc json jsonl json5 toml yaml yml ini cfg conf env log csv tsv xml html htm css scss " +
    "less svg png jpg jpeg gif webp bmp ico pdf rs ts tsx mts cts js jsx mjs cjs py pyi ipynb rb go java kt kts swift " +
    "c h hpp hh cpp cc cxx cs php sh bash zsh fish ps1 psm1 bat cmd sql graphql gql proto lua r jl dart ex exs erl " +
    "hrl zig nim vue svelte astro tf tfvars hcl nix gradle lock patch diff wasm"
  ).split(" "),
);

/** Names without an extension that are files. */
const NAMES = new Set(
  (
    "makefile dockerfile containerfile readme license licence changelog gemfile rakefile procfile justfile " +
    "codeowners .env .gitignore .gitattributes .dockerignore .editorconfig .npmrc .nvmrc .prettierrc .eslintrc"
  ).split(" "),
);

const SCHEME = /^[a-z][a-z0-9+.-]*:/i;
const DRIVE = /^[A-Za-z]:[\\/]/;
const LINE_SUFFIX = /(?::(\d{1,7})(?::(\d{1,5}))?|#L(\d{1,7})(?:-L?\d{1,7})?)$/;
const LEADING = /^[(["'`<{*_]+/;
const TRAILING = /[)\]"'`>}.,;:!?*_]+$/;
const HOST_PORT =
  /^(?:localhost|\d{1,3}(?:\.\d{1,3}){3}|\[[0-9a-f:]+\]|[a-z0-9-]+(?:\.[a-z0-9-]+)+):\d+/i;
/** Longest candidate considered. */
const MAX_LENGTH = 1024;

/** The last path segment of `path`. */
function lastSegment(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? "";
}

/** True when a file NAME (last segment) looks like a file. */
function looksLikeFileName(name: string): boolean {
  const lower = name.toLowerCase();
  if (NAMES.has(lower)) return true;
  const dot = lower.lastIndexOf(".");
  // A leading dot alone (`.env`) is handled by NAMES; `a.` or `.` is not a name.
  if (dot <= 0 || dot === lower.length - 1) return false;
  return EXTENSIONS.has(lower.slice(dot + 1));
}

/** Splits a `:12` / `#` suffix off `text`. */
function splitLine(text: string): { path: string; line: number | null } {
  // A Windows drive (`C:`) is not a line suffix.
  const match = LINE_SUFFIX.exec(text);
  if (match === null || match.index === 0) return { path: text, line: null };
  const digits = match[1] ?? match[3];
  return { path: text.slice(0, match.index), line: digits === undefined ? null : Number(digits) };
}

/**
 * The path a token names, or `null`. `relaxed`: the token came from inline code or a link target, where any path
 * shape with a folder part counts (spaces inside allowed by the caller).
 */
export function pathOf(
  token: string,
  relaxed = false,
): { path: string; line: number | null } | null {
  let text = token.trim();
  if (text === "" || text.length > MAX_LENGTH) return null;
  if (/^file:\/\//i.test(text)) {
    try {
      const url = new URL(text);
      if (url.host !== "") return null;
      text = decodeURIComponent(url.pathname).replace(/^\/([A-Za-z]:[\\/])/, "$1");
    } catch {
      return null;
    }
  } else if (SCHEME.test(text) && !DRIVE.test(text)) {
    return null;
  }
  if (HOST_PORT.test(text) || /^www\./i.test(text)) return null;
  if (text.includes("@") && !/[\\/]/.test(text)) return null;
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f<>|"*?]/.test(text)) return null;
  const { path, line } = splitLine(text);
  if (path === "" || path === "/" || path.startsWith("//") || path.startsWith("\\\\")) return null;
  const name = lastSegment(path);
  if (name === "" || name === "." || name === "..") return null;
  const anchored =
    path.startsWith("/") ||
    path.startsWith("~/") ||
    path.startsWith("./") ||
    path.startsWith("../") ||
    DRIVE.test(path);
  const hasFolder = /[\\/]/.test(path);
  if (anchored) return { path, line };
  if (hasFolder && (relaxed || looksLikeFileName(name))) return { path, line };
  if (!hasFolder && looksLikeFileName(name)) return { path, line };
  return null;
}

/** Paths in plain text (no Markdown), at most `max`, in order. */
export function findPathRefs(text: string, max = 100): PathRef[] {
  const refs: PathRef[] = [];
  const token = /[^\s]+/g;
  let match: RegExpExecArray | null;
  while ((match = token.exec(text)) !== null && refs.length < max) {
    const raw = match[0];
    const lead = LEADING.exec(raw)?.[0].length ?? 0;
    let body = raw.slice(lead);
    // Trailing punctuation (`a.ts:12.` keeps `:12`; `a.ts:` drops the colon).
    body = body.replace(TRAILING, "");
    const found = pathOf(body);
    if (found === null) continue;
    const start = match.index + lead;
    refs.push({ start, end: start + body.length, path: found.path, line: found.line });
  }
  return refs;
}

/** The path an inline code span names (its whole content), or `null`. Spaces are allowed only when the last part
 * is clearly a file name (`My Docs/report.md`), so a command like `cd src && make` is not a path. */
export function pathOfCode(code: string): { path: string; line: number | null } | null {
  const text = code.trim();
  if (/\s/.test(text) && !looksLikeFileName(lastSegment(splitLine(text).path))) return null;
  return pathOf(text, true);
}

/** The path a Markdown link target names (relative, absolute or `file://`), or `null` for web links and anchors. */
export function pathOfHref(href: string): { path: string; line: number | null } | null {
  if (href === "" || href.startsWith("#") || href.startsWith("?")) return null;
  let decoded = href;
  if (!/^file:/i.test(href)) {
    try {
      decoded = decodeURI(href);
    } catch {
      return null;
    }
  }
  return pathOf(decoded, true);
}

/** A relative Markdown folder target. The UI only links it after a status result names that exact repo root. */
export function repoFolderOfHref(href: string): string | null {
  if (!href.endsWith("/") || href.startsWith("/") || href.startsWith("//") || href.includes("?"))
    return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(href);
  } catch {
    return null;
  }
  if (/^[a-z][a-z0-9+.-]*:/i.test(decoded) || decoded.includes("\\") || decoded.includes("#"))
    return null;
  const root = decoded.replace(/^\.\//, "").replace(/\/$/, "");
  if (root === "") return ".";
  if (root.split("/").some((part) => part === "" || part === "." || part === "..")) return null;
  return root;
}
