/**
 * The code editor colours the same languages as the read-only file view: both offer the same language names,
 * every file name the view colours opens in the editor as that same language, and the editor really highlights it.
 * Uses the real CodeMirror editor in jsdom (support/codeEditor supplies the browser APIs jsdom lacks).
 */
import { render, waitFor } from "@testing-library/react";
import { beforeAll, describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { ensureSyntaxTree, language } from "@codemirror/language";
import { highlightTree } from "@lezer/highlight";

import {
  HIGHLIGHT_LANGUAGES,
  languageOf,
} from "../../../ui/src/components/markdown/highlight";
import { languageForName } from "../../../ui/src/features/files/model/kinds";
import { CodeEditor } from "../../../ui/src/features/editor/CodeEditor";
import {
  EDITOR_LANGUAGES,
  languageFor,
} from "../../../ui/src/features/editor/languages";
import { editorHighlight } from "../../../ui/src/features/editor/theme";
import { setupCodeEditor } from "./support/codeEditor";

beforeAll(setupCodeEditor);

/**
 * The editor's language name for each language the view offers. C files use the C++ grammar (C is close to a
 * subset of it), so both `c` and `cpp` open as `cpp`. CodeMirror names its JSX dialects after their base
 * language, so `jsx` and `tsx` are checked by their JSX syntax tree as well.
 */
const EDITOR_NAME: Record<string, string> = {
  bash: "shell",
  c: "cpp",
  cpp: "cpp",
  csharp: "csharp",
  css: "css",
  diff: "diff",
  dockerfile: "dockerfile",
  go: "go",
  html: "html",
  java: "java",
  javascript: "javascript",
  json: "json",
  jsx: "javascript",
  kotlin: "kotlin",
  markdown: "markdown",
  php: "php",
  powershell: "powershell",
  python: "python",
  ruby: "ruby",
  rust: "rust",
  sql: "sql",
  swift: "swift",
  toml: "toml",
  tsx: "typescript",
  typescript: "typescript",
  xml: "xml",
  yaml: "yaml",
};

/** Real file names and a line of each language; every language the view offers has at least one. */
const FILES: readonly (readonly [name: string, source: string])[] = [
  ["run.sh", 'echo "hello"'],
  ["build.bash", 'echo "hello"'],
  ["init.zsh", 'echo "hello"'],
  ["Makefile", 'all:\n\techo "hello"'],
  [".env", 'NAME="value"'],
  ["main.c", "int main(void) { return 0; }"],
  ["main.h", "int answer(void);"],
  ["main.cpp", "int main() { return 0; }"],
  ["main.hpp", "class A {};"],
  ["main.cc", "int main() { return 0; }"],
  ["Program.cs", "class A { int x = 42; }"],
  ["site.css", ".x { color: red; }"],
  ["site.scss", ".x { color: red; }"],
  ["site.less", ".x { color: red; }"],
  ["fix.diff", "@@ -1 +1 @@\n-old\n+new"],
  ["fix.patch", "@@ -1 +1 @@\n-old\n+new"],
  ["Dockerfile", "FROM alpine\nRUN echo hello"],
  ["Containerfile", "FROM alpine\nRUN echo hello"],
  ["main.go", "package main\n\nfunc main() { var x = 42 }"],
  ["page.html", '<div title="x">Hi</div>'],
  ["page.htm", '<div title="x">Hi</div>'],
  ["Main.java", "class Main { int x = 42; }"],
  ["app.js", "const answer = 42"],
  ["app.mjs", "const answer = 42"],
  ["app.cjs", "const answer = 42"],
  ["data.json", '{"answer": 42}'],
  ["data.jsonc", '{"answer": 42}'],
  ["view.jsx", "const value = <div>Hello</div>"],
  ["Main.kt", "fun main() { val x = 42 }"],
  ["build.gradle.kts", "val x = 42"],
  ["README.md", "# Title\n\n[site](https://example.com)"],
  ["guide.markdown", "[site](https://example.com)"],
  ["index.php", "<?php echo 42; ?>"],
  ["run.ps1", '$name = "value"'],
  ["tool.psm1", '$name = "value"'],
  ["main.py", "def f():\n  return 42"],
  ["stubs.pyi", "def f() -> int: ..."],
  ["app.rb", "def f\n  42\nend"],
  ["main.rs", "fn main() { let x = 42; }"],
  ["query.sql", "SELECT name FROM users WHERE id = 42"],
  ["App.swift", "let answer = 42"],
  ["Cargo.toml", "answer = 42"],
  ["view.tsx", "const value: string = <div>Hello</div>"],
  ["app.ts", "const answer: number = 42"],
  ["app.mts", "const answer: number = 42"],
  ["app.cts", "const answer: number = 42"],
  ["pom.xml", '<project version="1"/>'],
  ["icon.svg", '<svg width="1"/>'],
  ["ci.yaml", "answer: 42"],
  ["ci.yml", "answer: 42"],
];

describe("editor and viewer languages", () => {
  it("both offer the same language names", () => {
    expect([...EDITOR_LANGUAGES].sort()).toEqual(
      [...HIGHLIGHT_LANGUAGES].sort(),
    );
    expect(Object.keys(EDITOR_NAME).sort()).toEqual(
      [...HIGHLIGHT_LANGUAGES].sort(),
    );
  });

  it("opens every file name the view colours as the same language, highlighted", async () => {
    const covered = new Set<string>();
    for (const [name, source] of FILES) {
      const viewed = languageOf(languageForName(name));
      if (viewed === null) throw new Error(`${name} is plain in the view`);
      covered.add(viewed);
      const state = EditorState.create({
        doc: source,
        extensions: [await languageFor(name)],
      });
      const editing = state.facet(language);
      expect(editing?.name, `${name} in the editor`).toBe(EDITOR_NAME[viewed]);
      const tree = ensureSyntaxTree(state, state.doc.length, 1000);
      if (tree === null) throw new Error(`${name} has no syntax tree`);
      if (viewed === "jsx" || viewed === "tsx") {
        const nodes: string[] = [];
        tree.iterate({ enter: (node) => void nodes.push(node.name) });
        expect(nodes, `${name} parses JSX`).toContain("JSXElement");
      }
      const classes: string[] = [];
      highlightTree(tree, editorHighlight, (_from, _to, className) =>
        classes.push(className),
      );
      expect(
        classes.some((c) => c.includes("syn-token-")),
        `${name} has coloured tokens`,
      ).toBe(true);
    }
    expect([...covered].sort()).toEqual([...HIGHLIGHT_LANGUAGES].sort());
  });

  it("leaves a file the view does not colour plain in the editor too", async () => {
    const names = [
      "notes.txt",
      "LICENSE",
      "data.bin",
      "md",
      "a.constructor",
      "constructor",
      "a.__proto__",
    ];
    for (const name of names) {
      expect(languageOf(languageForName(name)), name).toBeNull();
      const state = EditorState.create({
        doc: "text",
        extensions: [await languageFor(name)],
      });
      expect(state.facet(language), name).toBeNull();
    }
  });

  it.each([
    ["main.go", "package main\n\nfunc main() {}", "go", "package"],
    ["Dockerfile", "FROM alpine\nRUN echo hello", "dockerfile", "FROM"],
  ])("colours %s on Edit", async (filename, value, name, keyword) => {
    const { container } = render(
      <CodeEditor
        value={value}
        onChange={() => {}}
        label="Code"
        filename={filename}
        line={null}
      />,
    );
    const root = container.querySelector("[data-code-editor]")?.shadowRoot;
    await waitFor(() =>
      expect(root?.querySelector(".cm-content")).toHaveAttribute(
        "data-language",
        name,
      ),
    );
    await waitFor(() =>
      expect(root?.querySelector(".syn-token-keyword")).toHaveTextContent(
        keyword,
      ),
    );
  });
});
