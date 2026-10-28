/**
 * The words and choices of the notice above an open editor after the file changed on disk (reloads, merges and
 * conflicts). One source for the notice the window draws and the `ui/editors` route's answer.
 * Depends on: state/fileEdits. Called by: DiskNotice, editorsRoute.
 * Invariants: pure; every sentence names the outcome that already happened (the editor reloaded, merged, or is
 * unchanged); "Vibe" is named only when the open-files mark says Vibe wrote the file, otherwise "on disk".
 */
import {
  acknowledgeDiskChange,
  resolveOverlap,
  undoMerge,
  type DiskNotice,
  type FileEdit,
} from "../../../state/fileEdits";

/** A choice on the notice (its buttons, and the route's `diskNotice {choice}`). */
export type DiskChoice = "gotIt" | "undoMerge" | "keepYours" | "takeVibes";

/** "line 8" or "lines 8 to 11" (first to last marked line). */
function linesText(lines: readonly number[]): string {
  const first = lines[0];
  const last = lines[lines.length - 1];
  if (first === undefined || last === undefined) return "some lines";
  return first === last ? `line ${String(first)}` : `lines ${String(first)} to ${String(last)}`;
}

/** The notice's sentence. `byVibe`: Vibe wrote the file (the open-files mark). */
export function noticeText(notice: DiskNotice, name: string, byVibe: boolean): string {
  const lines = linesText(notice.lines);
  switch (notice.kind) {
    case "reloaded":
      return byVibe
        ? `Vibe changed ${lines} while you were looking.`
        : `${name} changed on disk (${lines}) while you were looking.`;
    case "merged":
      if (notice.chose === "mine")
        return byVibe
          ? "You kept your lines where you and Vibe changed the same lines. Vibe's other changes are in this editor too; Save writes the result."
          : "You kept your lines where both versions changed the same lines. The other changes on disk are in this editor too; Save writes the result.";
      if (notice.chose === "theirs")
        return byVibe
          ? "You took Vibe's lines where you both changed the same lines. Your other edits stay; Save writes the result."
          : "You took the lines on disk where both versions changed the same lines. Your other edits stay; Save writes the result.";
      return `${byVibe ? "Vibe" : "The file on disk"} changed ${lines} while you had unsaved edits on other lines. Both are in this editor now; Save writes them.`;
    case "overlap":
      return byVibe
        ? `Vibe changed ${name} while you had unsaved edits, and you both changed ${lines}. Nothing is lost yet: your text is still in this editor and Vibe's is on disk.`
        : `${name} changed on disk while you had unsaved edits, and both changed ${lines}. Nothing is lost yet: your text is still in this editor and the new version is on disk.`;
  }
}

/** The label of the "take the disk side" button. */
export function takeLabel(byVibe: boolean): string {
  return byVibe ? "Take Vibe's" : "Take the version on disk";
}

/** The choices the notice offers now (Compare side by side is the window's, it opens the split). */
export function choicesOf(notice: DiskNotice | null): DiskChoice[] {
  if (notice === null) return [];
  if (notice.kind === "reloaded") return ["gotIt"];
  if (notice.kind === "merged") return ["undoMerge", "gotIt"];
  return ["keepYours", "takeVibes"];
}

/** The draft after `choice`. */
export function applyChoice(draft: FileEdit, choice: DiskChoice): FileEdit {
  switch (choice) {
    case "gotIt":
      return acknowledgeDiskChange(draft);
    case "undoMerge":
      return undoMerge(draft);
    case "keepYours":
      return resolveOverlap(draft, "mine");
    case "takeVibes":
      return resolveOverlap(draft, "theirs");
  }
}
