/**
 * ESLint flat config for the UI: typescript-eslint (strict, type-checked) plus react-hooks.
 * Run by `pnpm lint` (zero warnings allowed) and by scripts/public/check.sh.
 */
import js from "@eslint/js";
import controlTestId from "./control-test-id.js";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  // src/protocol/generated (tools/public/codegen) and src/protocol/core (crates/vw-core-api,
  // gen-core-ts) are machine-written and checked for drift instead.
  { ignores: ["dist/", "node_modules/", "src/protocol/generated/", "src/protocol/core/"] },
  js.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  reactHooks.configs.flat.recommended,
  {
    rules: {
      "no-restricted-properties": [
        "error",
        ...["toSorted", "toReversed", "toSpliced", "with", "findLast", "findLastIndex"].map(
          (property) => ({
            property,
            message: "Use older array methods for the macOS 11 system WebKit (Safari 14).",
          }),
        ),
      ],
    },
    languageOptions: {
      globals: { ...globals.browser },
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
    },
  },
  {
    files: ["src/**/*.tsx"],
    plugins: { controls: { rules: { "test-id": controlTestId } } },
    rules: { "controls/test-id": "error" },
  },
  // Plain JS files (this config) are not part of the TypeScript project: lint them without type info.
  { files: ["**/*.js"], ...tseslint.configs.disableTypeChecked },
);
