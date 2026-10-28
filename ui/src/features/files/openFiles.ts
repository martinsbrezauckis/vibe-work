/**
 * The open-files words shared by the file tabs, the +N open-files list and the `ui/editors` route: a tab's short
 * name, its marks (Unsaved, Vibe changed it) and the rows the list shows. Pure reads of the file workspace.
 * Depends on: state/files (the workspace controller). Called by: shell/FilePanelTabs, files/editorsRoute.
 * Invariant: the list shows every open document in workspace order; the route answers the same rows the window
 * draws, so a Debug API caller and a person read the same words.
 */
import type { Files } from "../../state/files";
import type { OpenDocument } from "../../state/fileWorkspace";

/** One row of the +N open-files list (and of the `ui/editors` state). */
export interface OpenFileRow {
  /** The document id (a file handle, or `saved:<handle>` / `disk:<handle>` for a read-only comparison side). */
  file: string;
  /** The tab's short name: the last path segment. */
  name: string;
  /** The path the tab's tooltip and the list's second line show. */
  display: string;
  /** `saved`, `disk` or `commit` (Files > History) for a read-only comparison side, else null. */
  saved: "saved" | "disk" | "commit" | null;
  /** The rendered preview beside the file's code. */
  preview: boolean;
  unsaved: boolean;
  changed: boolean;
  /** "Unsaved", "Vibe changed it", both joined by " · ", or null. */
  mark: string | null;
  /** Shown as a tab in the strip (the rest are behind +N). */
  visible: boolean;
  /** The focused document. */
  active: boolean;
}

/** The short name a file tab shows for `file` in `sessionId` (the last path segment). */
export function openFileName(files: Files, sessionId: string, file: string): string {
  return files.workspace.of(sessionId).documents[file]?.shown.display.split(/[\\/]/).pop() ?? file;
}

/** The marks after a file's name in the open-files list, or null when it has none. */
export function openFileMark(files: Files, sessionId: string, file: string): string | null {
  return (
    [
      files.workspace.dirty(sessionId, file) ? "Unsaved" : null,
      files.workspace.of(sessionId).documents[file]?.changed === true ? "Vibe changed it" : null,
    ]
      .filter(Boolean)
      .join(" · ") || null
  );
}

/** Every open document of `sessionId` as the +N list shows it, in workspace order. */
export function openFileRows(files: Files, sessionId: string): OpenFileRow[] {
  const view = files.workspace.of(sessionId);
  return view.order.map((file) => {
    const doc = view.documents[file];
    return {
      file,
      name: openFileName(files, sessionId, file),
      display: doc?.shown.display ?? file,
      saved: doc?.saved === undefined ? null : (doc.savedName ?? "saved"),
      preview: doc?.preview === true,
      unsaved: files.workspace.dirty(sessionId, file),
      changed: doc?.changed === true,
      mark: openFileMark(files, sessionId, file),
      visible: view.visible.includes(file),
      active: view.active === file,
    };
  });
}

/** The read-only side's words: its short tab label and the line above its editor. */
export function savedWords(doc: OpenDocument | undefined): { tab: string; line: string } {
  if (doc?.savedName === "commit")
    return {
      tab: doc.savedLabel ?? "commit",
      line: `${doc.savedLabel ?? "commit"} · as it was in this commit, read-only`,
    };
  if (doc?.savedName === "disk") return { tab: "disk", line: "disk · read-only" };
  return { tab: "saved", line: "saved · last saved version, read-only" };
}
