/**
 * Lazy CodeMirror language selection by file name; called by CodeEditor and MergeCompare.
 * A file opens as the language the read-only file view colours it as: the name goes through the view's own
 * resolution (`languageForName`, then `languageOf`), and every language the view offers has a loader here, so a
 * file never turns plain on Edit. A name the view leaves plain stays plain here too.
 * Each loader is a dynamic import, its own chunk fetched on first use. Languages without a CodeMirror 6 package use
 * `@codemirror/legacy-modes` stream parsers. Loaders return the bare language (no completion or auto-close
 * extensions), so typing behaves the same in every language.
 * Depends on: @codemirror/lang-*, @codemirror/legacy-modes, components/markdown/highlight, files/model/kinds.
 */
import type { StreamParser } from "@codemirror/language";
import type { Extension } from "@codemirror/state";
import { type HighlightLanguage, languageOf } from "../../components/markdown/highlight";
import { languageForName } from "../files/model/kinds";

/** A legacy-mode language. `name` labels a mode that does not name itself (the editor shows it as `data-language`). */
async function legacy<T>(mode: Promise<StreamParser<T>>, name?: string): Promise<Extension> {
  const [{ StreamLanguage }, parser] = await Promise.all([import("@codemirror/language"), mode]);
  return StreamLanguage.define(name === undefined ? parser : { ...parser, name });
}

/**
 * One loader per language the view offers; the type makes a language added to the view fail to compile until it is
 * added here. C uses the C++ grammar (CodeMirror has no separate C package and C reads well as C++).
 */
const LOADERS: Record<HighlightLanguage, () => Promise<Extension>> = {
  bash: () => legacy(import("@codemirror/legacy-modes/mode/shell").then((m) => m.shell)),
  c: async () => (await import("@codemirror/lang-cpp")).cppLanguage,
  cpp: async () => (await import("@codemirror/lang-cpp")).cppLanguage,
  csharp: () => legacy(import("@codemirror/legacy-modes/mode/clike").then((m) => m.csharp)),
  css: async () => (await import("@codemirror/lang-css")).cssLanguage,
  diff: () => legacy(import("@codemirror/legacy-modes/mode/diff").then((m) => m.diff)),
  dockerfile: () =>
    legacy(
      import("@codemirror/legacy-modes/mode/dockerfile").then((m) => m.dockerFile),
      "dockerfile",
    ),
  go: async () => (await import("@codemirror/lang-go")).goLanguage,
  html: async () => (await import("@codemirror/lang-html")).htmlLanguage,
  java: async () => (await import("@codemirror/lang-java")).javaLanguage,
  javascript: async () => (await import("@codemirror/lang-javascript")).javascriptLanguage,
  json: async () => (await import("@codemirror/lang-json")).json(),
  jsx: async () => (await import("@codemirror/lang-javascript")).jsxLanguage,
  kotlin: () => legacy(import("@codemirror/legacy-modes/mode/clike").then((m) => m.kotlin)),
  markdown: async () => (await import("@codemirror/lang-markdown")).markdownLanguage,
  php: async () => {
    const [{ php }, { htmlLanguage }] = await Promise.all([
      import("@codemirror/lang-php"),
      import("@codemirror/lang-html"),
    ]);
    // HTML around the `<?php` blocks is coloured too; passing the bare HTML language leaves out its auto-close.
    return php({ baseLanguage: htmlLanguage });
  },
  powershell: () =>
    legacy(import("@codemirror/legacy-modes/mode/powershell").then((m) => m.powerShell)),
  python: async () => (await import("@codemirror/lang-python")).pythonLanguage,
  ruby: () => legacy(import("@codemirror/legacy-modes/mode/ruby").then((m) => m.ruby)),
  rust: async () => (await import("@codemirror/lang-rust")).rust(),
  sql: async () => (await import("@codemirror/lang-sql")).StandardSQL.language,
  swift: () => legacy(import("@codemirror/legacy-modes/mode/swift").then((m) => m.swift)),
  toml: () => legacy(import("@codemirror/legacy-modes/mode/toml").then((m) => m.toml)),
  tsx: async () => (await import("@codemirror/lang-javascript")).tsxLanguage,
  typescript: async () => (await import("@codemirror/lang-javascript")).typescriptLanguage,
  xml: async () => (await import("@codemirror/lang-xml")).xmlLanguage,
  yaml: async () => (await import("@codemirror/lang-yaml")).yaml(),
};

/** Every language the editor colours: the same ids as the view's `HIGHLIGHT_LANGUAGES`. */
export const EDITOR_LANGUAGES = Object.keys(LOADERS) as readonly HighlightLanguage[];

/** The editor language for a file name, or no extension (plain text) when the view does not colour it either. */
export async function languageFor(filename: string): Promise<Extension> {
  const id = languageOf(languageForName(filename));
  return id === null ? [] : LOADERS[id]();
}
