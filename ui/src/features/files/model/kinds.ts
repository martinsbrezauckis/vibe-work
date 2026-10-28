/**
 * File names to highlighting languages and sizes to words for the File tab. Pure.
 * The language is only a hint for the highlighter (`components/markdown/highlight.ts` `languageOf`, which ignores a
 * name it does not offer); what a file IS comes from the core (`FileContent.kind`, decided from the bytes).
 * A name without a dot is not its own extension (a file named `md` is not Markdown).
 * Depends on: nothing. Called by: FileViews, FileTab, the code editor's `languageFor`, tests.
 */

const BY_NAME: Record<string, string> = {
  dockerfile: "dockerfile",
  containerfile: "dockerfile",
  makefile: "bash",
  ".env": "bash",
  ".bashrc": "bash",
  ".zshrc": "bash",
};

const BY_EXT: Record<string, string> = {
  h: "c",
  hpp: "cpp",
  hh: "cpp",
  cc: "cpp",
  cxx: "cpp",
  kts: "kotlin",
  kt: "kotlin",
  jsonl: "json",
  json5: "json",
  jsonc: "json",
  htm: "html",
  markdown: "md",
  mdx: "md",
  bash: "bash",
  zsh: "bash",
  env: "bash",
  psm1: "ps1",
  pyi: "py",
  scss: "css",
  less: "css",
};

/** `table[key]` for an own key only, so a name like `constructor` or `__proto__` is never a table entry. */
function entry(table: Record<string, string>, key: string): string | undefined {
  return Object.hasOwn(table, key) ? table[key] : undefined;
}

/**
 * The fence-style language name for a file name (`languageOf` maps it to a loaded grammar), or `null`. The code
 * editor resolves names through this too, so viewing and editing a file pick the same language.
 */
export function languageForName(name: string): string | null {
  const lower = name.toLowerCase();
  const named = entry(BY_NAME, lower);
  if (named !== undefined) return named;
  const dot = lower.lastIndexOf(".");
  if (dot <= 0 || dot === lower.length - 1) return null;
  const ext = lower.slice(dot + 1);
  return entry(BY_EXT, ext) ?? ext;
}

/** A size in words: `512 B`, `12 KB`, `3.4 MB`. */
export function sizeText(bytes: number): string {
  if (bytes < 1024) return `${String(bytes)} B`;
  if (bytes < 1024 * 1024) return `${String(Math.ceil(bytes / 1024))} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
