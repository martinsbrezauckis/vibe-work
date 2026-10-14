/**
 * The palette rows for the core's search results: messages open the conversation at the match,
 * files open the File tab at the line.
 * Depends on: model (paths, open actions), paletteTypes, useCoreSearch (`CoreResults`). Called by: CommandPalette.
 */
import type { App } from "../shell/AppContext";
import { filePath, messagePath, openFile, openMessage } from "./model";
import type { Result } from "./paletteTypes";
import type { CoreResults } from "./useCoreSearch";

/** Message and file rows for `results` (empty while nothing is typed). */
export function coreRows(app: App, results: CoreResults): { message: Result[]; file: Result[] } {
  const message: Result[] = results.messages.map((hit) => ({
    id: `message:${hit.sessionId}:${hit.entryId}`,
    kind: "message",
    title: hit.snippet,
    path: messagePath(hit),
    detail: hit.snippet,
    opens: "Opens the conversation at this message, with find in this conversation",
    run: () => void openMessage(app, hit, results.query),
  }));
  const file: Result[] = results.files.map((hit) => ({
    id: `file:${hit.folder}:${hit.path}:${String(hit.line ?? 0)}`,
    kind: "file",
    title: hit.line === null ? hit.path : hit.text,
    path: filePath(hit),
    detail: hit.line === null ? "The file's name matches" : hit.text,
    opens:
      hit.line === null
        ? "Opens the file in the File tab"
        : `Opens the file in the File tab at line ${String(hit.line)}`,
    run: () => {
      void openFile(app, hit).then((opened) => {
        if (!opened)
          app.toasts.push({
            kind: "warn",
            bellKind: "failure",
            text: "That file can't be opened here: no session works in its folder any more.",
          });
      });
    },
  }));
  return { message, file };
}

/** The line under the result list while the message index fills, or `null`. */
export function indexNote(results: CoreResults): string | null {
  const index = results.index;
  if (index === null) return null;
  if (index.error !== null) return `Older conversations could not be searched: ${index.error}`;
  if (!index.filling) return null;
  return `Searching older conversations… (${String(index.indexed)} of ${String(index.total)})`;
}
