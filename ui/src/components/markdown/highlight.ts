/**
 * Syntax highlighting for code blocks in model output, CSP-safe:
 * - `shiki/core` with `shiki/engine/javascript`: no WebAssembly, so
 *   the CSP needs no `wasm-unsafe-eval`;
 * - languages are imported lazily, one chunk each, from a fixed list (an unknown language is plain
 *   text, never a dynamic import of an arbitrary name);
 * - shiki's CSS-variables theme: every token colour is `var(--shiki-token-…)`, mapped here to a class
 *   (`syn-token-keyword`) whose colour is a design token (`--syn-*` in `styles/tokens.css`). Tokens are
 *   rendered as React elements with classes only: no HTML string, no `style` attribute
 *   (`codeToHtml` is forbidden: it needs innerHTML and its `style="…"` is blocked by `style-src 'self'`).
 * Depends on: shiki 4.5.0. Called by: CodeBlock, the File tab, and the code
 * editor's language list (`features/editor/languages.ts`), which offers exactly the languages listed here.
 * Invariants: never throws to the caller (a failure means plain text); large inputs are not
 * highlighted (`MAX_HIGHLIGHT_CHARS`); results are cached per (language, code) for re-mounted rows.
 */
import type { HighlighterCore, LanguageInput } from "shiki/core";

/** Largest code block that is highlighted; bigger ones stay plain text (keeps the window responsive). */
const MAX_HIGHLIGHT_CHARS = 30_000;

/** One highlighted piece of a line. */
export interface CodeToken {
  text: string;
  /** Space-separated classes (`syn-token-keyword syn-italic`), "" for plain text. */
  className: string;
}

type Loader = () => Promise<{ default: LanguageInput }>;

/** The languages offered, by shiki id. Each import becomes its own lazily loaded chunk. */
const LANGUAGES = {
  bash: () => import("shiki/langs/shellscript.mjs"),
  c: () => import("shiki/langs/c.mjs"),
  cpp: () => import("shiki/langs/cpp.mjs"),
  csharp: () => import("shiki/langs/csharp.mjs"),
  css: () => import("shiki/langs/css.mjs"),
  diff: () => import("shiki/langs/diff.mjs"),
  dockerfile: () => import("shiki/langs/dockerfile.mjs"),
  go: () => import("shiki/langs/go.mjs"),
  html: () => import("shiki/langs/html.mjs"),
  java: () => import("shiki/langs/java.mjs"),
  javascript: () => import("shiki/langs/javascript.mjs"),
  json: () => import("shiki/langs/json.mjs"),
  jsx: () => import("shiki/langs/jsx.mjs"),
  kotlin: () => import("shiki/langs/kotlin.mjs"),
  markdown: () => import("shiki/langs/markdown.mjs"),
  php: () => import("shiki/langs/php.mjs"),
  powershell: () => import("shiki/langs/powershell.mjs"),
  python: () => import("shiki/langs/python.mjs"),
  ruby: () => import("shiki/langs/ruby.mjs"),
  rust: () => import("shiki/langs/rust.mjs"),
  sql: () => import("shiki/langs/sql.mjs"),
  swift: () => import("shiki/langs/swift.mjs"),
  toml: () => import("shiki/langs/toml.mjs"),
  tsx: () => import("shiki/langs/tsx.mjs"),
  typescript: () => import("shiki/langs/typescript.mjs"),
  xml: () => import("shiki/langs/xml.mjs"),
  yaml: () => import("shiki/langs/yaml.mjs"),
} satisfies Record<string, Loader>;

/** A language id this module offers. */
export type HighlightLanguage = keyof typeof LANGUAGES;

/** Every language id offered; the code editor offers the same ones. */
export const HIGHLIGHT_LANGUAGES = Object.keys(LANGUAGES) as readonly HighlightLanguage[];

/** Whether `id` is an offered language: an own key only (a name like `constructor` is not a language). */
function offered(id: string): id is HighlightLanguage {
  return Object.hasOwn(LANGUAGES, id);
}

/** Fence names that mean one of the languages above. */
const ALIASES: Record<string, string> = {
  sh: "bash",
  shell: "bash",
  shellscript: "bash",
  zsh: "bash",
  console: "bash",
  "c++": "cpp",
  cs: "csharp",
  "c#": "csharp",
  golang: "go",
  js: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  md: "markdown",
  ps1: "powershell",
  py: "python",
  rb: "ruby",
  rs: "rust",
  ts: "typescript",
  mts: "typescript",
  cts: "typescript",
  yml: "yaml",
  svg: "xml",
  patch: "diff",
};

/** The language id for a fence name, or `null` when it is not offered. */
export function languageOf(fence: string | null): HighlightLanguage | null {
  if (fence === null) return null;
  const name = fence.trim().toLowerCase();
  const id = (Object.hasOwn(ALIASES, name) ? ALIASES[name] : undefined) ?? name;
  return offered(id) ? id : null;
}

const THEME = "vw-tokens";
let highlighter: Promise<HighlighterCore> | null = null;
const loaded = new Map<string, Promise<boolean>>();
const cache = new Map<string, CodeToken[][]>();
const CACHE_LIMIT = 200;

function getHighlighter(): Promise<HighlighterCore> {
  highlighter ??= Promise.all([import("shiki/core"), import("shiki/engine/javascript")]).then(
    ([core, engine]) =>
      core.createHighlighterCore({
        themes: [
          core.createCssVariablesTheme({
            name: THEME,
            variablePrefix: "--shiki-",
            fontStyle: true,
          }),
        ],
        langs: [],
        // `forgiving`: a grammar pattern the JS engine cannot express is skipped, not fatal.
        engine: engine.createJavaScriptRegexEngine({ forgiving: true }),
      }),
  );
  return highlighter;
}

function loadLanguage(hl: HighlighterCore, id: string): Promise<boolean> {
  let pending = loaded.get(id);
  if (pending === undefined) {
    const loader: Loader | undefined = offered(id) ? LANGUAGES[id] : undefined;
    pending =
      loader === undefined
        ? Promise.resolve(false)
        : loader()
            .then(async (mod) => {
              await hl.loadLanguage(mod.default);
              return true;
            })
            .catch(() => false);
    loaded.set(id, pending);
  }
  return pending;
}

const VAR = /^var\(--shiki-([a-z-]+)\)$/;

function classOf(color: string | undefined, fontStyle: number | undefined): string {
  const names: string[] = [];
  const match = color === undefined ? null : VAR.exec(color);
  if (match?.[1] !== undefined && match[1] !== "foreground") names.push(`syn-${match[1]}`);
  if (fontStyle !== undefined && fontStyle > 0) {
    if ((fontStyle & 1) !== 0) names.push("syn-italic");
    if ((fontStyle & 2) !== 0) names.push("syn-bold");
    if ((fontStyle & 4) !== 0) names.push("syn-underline");
  }
  return names.join(" ");
}

/** Highlighted lines from the cache, or `null` when not computed yet. */
export function cachedTokens(code: string, lang: string): CodeToken[][] | null {
  return cache.get(`${lang}\u0000${code}`) ?? null;
}

/**
 * Highlights `code` as `lang` (a value from `languageOf`). Resolves to `null` when it cannot (unknown
 * language, larger than `limit`, a grammar failure): the caller keeps the plain text. `limit` defaults to
 * `MAX_HIGHLIGHT_CHARS` (chat code blocks); the File tab passes its own.
 */
export async function highlight(
  code: string,
  lang: string,
  limit = MAX_HIGHLIGHT_CHARS,
): Promise<CodeToken[][] | null> {
  if (code.length > limit || !offered(lang)) return null;
  const key = `${lang}\u0000${code}`;
  const hit = cache.get(key);
  if (hit !== undefined) return hit;
  try {
    const hl = await getHighlighter();
    if (!(await loadLanguage(hl, lang))) return null;
    const lines = hl.codeToTokensBase(code, { lang, theme: THEME }).map((line) =>
      line.map((token) => ({
        text: token.content,
        className: classOf(token.color, token.fontStyle),
      })),
    );
    if (cache.size >= CACHE_LIMIT) {
      const oldest = cache.keys().next().value;
      if (oldest !== undefined) cache.delete(oldest);
    }
    cache.set(key, lines);
    return lines;
  } catch {
    return null;
  }
}
