/** Lazy CodeMirror language selection by filename; called by CodeEditor and MergeCompare. */
import type { Extension } from "@codemirror/state";

export async function languageFor(filename: string): Promise<Extension> {
  const suffix = filename.split(".").pop()?.toLowerCase() ?? "";
  switch (suffix) {
    case "js":
    case "jsx":
    case "mjs":
    case "cjs":
      return suffix === "jsx"
        ? (await import("@codemirror/lang-javascript")).jsxLanguage
        : (await import("@codemirror/lang-javascript")).javascriptLanguage;
    case "ts":
    case "tsx":
    case "mts":
    case "cts":
      return suffix === "tsx"
        ? (await import("@codemirror/lang-javascript")).tsxLanguage
        : (await import("@codemirror/lang-javascript")).typescriptLanguage;
    case "json":
    case "jsonc":
      return (await import("@codemirror/lang-json")).json();
    case "css":
    case "scss":
      return (await import("@codemirror/lang-css")).cssLanguage;
    case "html":
    case "htm":
      return (await import("@codemirror/lang-html")).htmlLanguage;
    case "md":
    case "markdown":
      return (await import("@codemirror/lang-markdown")).markdownLanguage;
    case "py":
      return (await import("@codemirror/lang-python")).pythonLanguage;
    case "rs":
      return (await import("@codemirror/lang-rust")).rust();
    case "yaml":
    case "yml":
      return (await import("@codemirror/lang-yaml")).yaml();
    case "toml": {
      const [{ StreamLanguage }, { toml }] = await Promise.all([
        import("@codemirror/language"),
        import("@codemirror/legacy-modes/mode/toml"),
      ]);
      return StreamLanguage.define(toml);
    }
    case "sh":
    case "bash":
    case "zsh": {
      const [{ StreamLanguage }, { shell }] = await Promise.all([
        import("@codemirror/language"),
        import("@codemirror/legacy-modes/mode/shell"),
      ]);
      return StreamLanguage.define(shell);
    }
    default:
      return [];
  }
}
