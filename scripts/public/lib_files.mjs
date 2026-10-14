/**
 * Shared helper for the repository checks: lists the files git considers part of the project.
 *
 * Role: one definition of "which files exist" for check_size.mjs and check_public_paths.mjs.
 * Depends on: the `git` binary and Node 20+. Called by: the two check scripts in this folder.
 *
 * listProjectFiles returns tracked files plus untracked files that are not ignored, so a new file is checked
 * before it is ever added or committed. Deleted-but-still-indexed files are dropped.
 * listTrackedFiles returns exactly what the repository tracks (used by the public-repository mode of
 * check_public_paths.mjs).
 */
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** Absolute path of the repository root (two levels above scripts/public/). */
export const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/**
 * Lists project files as repo-relative POSIX paths, sorted.
 * @returns {string[]} tracked + untracked-not-ignored files that exist on disk.
 * @throws if git cannot be run (not a repository, git missing).
 */
export function listProjectFiles() {
  const out = execFileSync(
    "git",
    ["ls-files", "-z", "--cached", "--others", "--exclude-standard"],
    { cwd: REPO_ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
  );
  const files = out.split("\0").filter((p) => p.length > 0 && existsSync(resolve(REPO_ROOT, p)));
  return [...new Set(files)].sort();
}

/**
 * Lists the files git tracks (the index), as repo-relative POSIX paths, sorted. Unlike listProjectFiles it
 * ignores untracked files and keeps tracked files that are missing from disk: it answers "what does the
 * repository contain", which is what a publication guard has to judge.
 * @returns {string[]} tracked paths.
 * @throws if git cannot be run (not a repository, git missing).
 */
export function listTrackedFiles() {
  const out = execFileSync("git", ["ls-files", "-z", "--cached"], {
    cwd: REPO_ROOT,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  return [...new Set(out.split("\0").filter((p) => p.length > 0))].sort();
}
