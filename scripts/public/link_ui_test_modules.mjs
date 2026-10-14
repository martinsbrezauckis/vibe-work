/**
 * Gives public UI tests the UI package's installed dependencies without another installation.
 * Depends on Node's filesystem API; called by the UI check and test scripts. Refuses real folders
 * and links to another target. Windows uses a junction, which needs no symlink privilege.
 */
import { lstatSync, mkdirSync, realpathSync, symlinkSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const target = realpathSync(resolve(root, "ui/node_modules"));
const link = resolve(root, "tests/public/ui/node_modules");
let existing;
try {
  existing = lstatSync(link);
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
if (existing) {
  if (!existing.isSymbolicLink() || realpathSync(link) !== target) {
    throw new Error(`${link} must be a link to the UI package's node_modules`);
  }
} else {
  mkdirSync(dirname(link), { recursive: true });
  symlinkSync(target, link, process.platform === "win32" ? "junction" : "dir");
}
