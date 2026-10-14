/**
 * Lints the public UI tests with the UI's rules, from the tests' own folder (where their eslint.config.mjs is).
 * Called by the UI package's lint script. A separate script instead of `cd ... && eslint` in package.json: npm runs
 * scripts through cmd on Windows, where a `cd` inside parentheses changes the folder for the rest of the line.
 * Depends on the UI package's installed ESLint; exits with ESLint's status.
 */
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const eslint = resolve(root, "ui/node_modules/eslint/bin/eslint.js");
const result = spawnSync(process.execPath, [eslint, ".", "--max-warnings", "0"], {
  cwd: resolve(root, "tests/public/ui"),
  stdio: "inherit",
});
if (result.error) throw result.error;
process.exit(result.status ?? 1);
