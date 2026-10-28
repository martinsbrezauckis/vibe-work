/** Session location words. Depends on Git and copy facts; called by the header chip and its tests. */
import type { GitStatusView, WorktreeInfo } from "../../protocol/core";
import { chipText } from "./model";

/** A display name only; full paths remain in the chip's tooltip and expansion. */
function folderName(path: string): string {
  return (
    path
      .replace(/[\\/]+$/, "")
      .split(/[\\/]/)
      .at(-1) || path
  );
}

/** Name the project, actual copy, current branch and original base without Git shorthand. */
export function locationText(
  project: string | null,
  cwd: string | null,
  info: WorktreeInfo | null,
  status: GitStatusView | undefined,
): string | null {
  const name = project ?? (cwd === null ? null : folderName(cwd));
  if (info !== null && info.state !== "ready")
    return [name, chipText(info)].filter(Boolean).join(" › ");
  // A ready copy can arrive before the Git read for its new cwd. The old folder's branch belongs to
  // the main checkout; use the recorded copy branch until Git has read the copy itself.
  const current = info !== null && status?.location !== "copy" ? undefined : status;
  const head = current?.repo === "." ? current.state?.head : null;
  const branch =
    head?.kind === "branch"
      ? head.name
      : (current?.repos.find((repo) => repo.root === ".")?.branch ?? info?.branch ?? null);
  if (info === null && branch === null && head?.kind !== "detached") return null;
  const copy = info === null ? null : `copy "${folderName(info.path ?? cwd ?? info.branch)}"`;
  const text = [name, copy, branch === null ? "Not on a branch" : `branch ${branch}`]
    .filter(Boolean)
    .join(" › ");
  return info?.base == null ? text : `${text} · from ${info.base}`;
}
