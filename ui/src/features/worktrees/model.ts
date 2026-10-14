/**
 * Worktree per session: pure rules the views use. Whether "Work in a new copy" starts on, the
 * remembered choice, the header chip text and the end-of-session summary lines.
 * Depends on: protocol types. Called by: state/worktrees.ts and the worktree views.
 * Invariants: no I/O except the remembered choice in localStorage, every access wrapped (storage can be
 * missing or blocked; the app works without it).
 */
import type {
  IncludeReport,
  LostKind,
  SkipReason,
  WorktreeInfo,
  WorktreeInspection,
} from "../../protocol/core";

const CHOICE_KEY = "vw.worktreeCopy";

/** The user's last "Work in a new copy" choice (`null` when never made or storage is unavailable). */
export function rememberedChoice(): boolean | null {
  try {
    const value = localStorage.getItem(CHOICE_KEY);
    return value === "on" ? true : value === "off" ? false : null;
  } catch {
    return null;
  }
}

/** Remembers the choice; a storage failure is ignored. */
export function rememberChoice(on: boolean): void {
  try {
    localStorage.setItem(CHOICE_KEY, on ? "on" : "off");
  } catch {
    // Storage unavailable: the default applies next time.
  }
}

/**
 * Existing copies start with the toggle off. Other folders default on when another live session works
 * in the repository, else use the user's last choice, else off.
 */
export function copyByDefault(
  othersLive: boolean,
  remembered: boolean | null,
  existingCopy = false,
): boolean {
  return !existingCopy && (othersLive || remembered === true);
}

/** The header chip text for a session's copy, e.g. `vibe/fix-login · from main`. */
export function chipText(info: WorktreeInfo): string {
  const from = info.base === null ? "" : ` · from ${info.base}`;
  switch (info.state) {
    case "pending":
      return `Making a copy on ${info.branch}…`;
    case "missing":
      return `${info.branch} · copy missing`;
    case "removed":
      return `${info.branch} · copy removed`;
    case "ready":
      return `${info.branch}${from}`;
  }
}

/** The lines of the end-of-session dialog, in plain words. */
export function endSummary(inspection: WorktreeInspection): string[] {
  const n = inspection.fileCount;
  const lines: string[] = [
    n === 0
      ? "Nothing unsaved goes with it."
      : `${String(n)} item${n === 1 ? " holds" : "s hold"} unsaved work that removing it discards:`,
  ];
  return lines;
}

/** What happens to the branch, in plain words. */
export function branchLine(inspection: WorktreeInspection): string {
  const name = inspection.branch ?? "its branch";
  return inspection.branchPlan.kind === "deleted"
    ? `The branch ${name} is deleted with it (every commit on it is in another branch).`
    : `The branch ${name} stays (${inspection.branchPlan.reason}).`;
}

/** The build output that goes with the folder, in plain words (empty when none). */
export function buildLine(inspection: WorktreeInspection): string {
  const n = inspection.buildOutput;
  return n === 0
    ? ""
    : `${String(n)} build output item${n === 1 ? "" : "s"} (such as node_modules) go with the folder too.`;
}

/** A listed path's kind, in plain words. */
export function kindText(kind: LostKind): string {
  switch (kind) {
    case "added":
      return "new";
    case "modified":
      return "changed";
    case "deleted":
      return "deleted";
    case "renamed":
      return "renamed";
    case "typeChanged":
      return "type changed";
    case "ignored":
      return "ignored file, not build output";
    case "repository":
      return "a repository of its own, not inspected";
  }
}

/** Why a file named by `.worktreeinclude` was not copied, in plain words. */
export function skipText(reason: SkipReason): string {
  switch (reason) {
    case "link":
      return "a link (never followed)";
    case "notRegular":
      return "not a regular file";
    case "unsafeName":
      return "a name that is not allowed";
    case "notText":
      return "a name that is not text";
    case "exists":
      return "already in the copy";
    case "unreadable":
      return "could not be read or written";
    case "changed":
      return "changed while copying";
    case "insideWorktree":
      return "inside another working tree";
  }
}

/**
 * The one-line summary of the `.worktreeinclude` copy: `plan` true before the copy ("Copies N files from
 * .worktreeinclude"), false after ("Copied N files from .worktreeinclude"). Empty when the repository has no
 * such file.
 */
export function includeLine(report: IncludeReport, plan: boolean): string {
  if (!report.found) return "";
  const n = report.files.length;
  const noun = `${String(n)} file${n === 1 ? "" : "s"}`;
  if (report.stopped !== null && n === 0) return "Nothing is copied from .worktreeinclude.";
  if (n === 0) return "No ignored file matches .worktreeinclude.";
  return `${plan ? "Copies" : "Copied"} ${noun} from .worktreeinclude`;
}
