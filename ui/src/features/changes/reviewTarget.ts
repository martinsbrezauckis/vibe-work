/** Transcript review targets matched to core-reported paths and hunks; used by the Changes store and views. */
import type { ChangesView, FileChange, Hunk } from "../../protocol/core";

/** A link remains pending until a fresh Changes view answers it. */
export interface ReviewTarget {
  path: string;
  line: number;
  ready: boolean;
}

/** Matches a diff path to a canonical Changes path, respecting Windows case and separators. */
export function reviewPath(view: ChangesView, path: string): string | null {
  const windows = /^(?:[a-z]:[\\/]|\\\\|\/\/)/i.test(view.folder ?? view.repoRoot ?? "");
  const key = (value: string) => {
    const slashes = value.replaceAll("\\", "/").replace(/^\.\//, "").replace(/\/$/, "");
    return windows ? slashes.toLowerCase() : slashes;
  };
  const target = key(path);
  const paths =
    view.mode === "effects"
      ? view.turns.flatMap((turn) => turn.files.map((file) => file.path))
      : view.files.map((file) => file.path);
  return (
    paths.find((file) =>
      [
        file,
        ...(view.folder === null ? [] : [`${view.folder}/${file}`]),
        ...(view.repoRoot === null ? [] : [`${view.repoRoot}/${file}`]),
      ].some((candidate) => key(candidate) === target),
    ) ?? null
  );
}

/** The new-side hunk containing the step's first changed line, or the first surviving hunk. */
export function reviewHunk(file: FileChange, line: number): Hunk | null {
  return (
    file.hunks.find(
      (hunk) => line >= hunk.newStart && line < hunk.newStart + Math.max(1, hunk.newLines),
    ) ??
    file.hunks[0] ??
    null
  );
}
