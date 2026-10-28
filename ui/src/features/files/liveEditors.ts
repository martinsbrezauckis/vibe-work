/**
 * The editors on screen now, by session and file, so a read of the side panel (`panel_read`) can take an editor's
 * selection and cursor line without storing every caret move. Depends on: CodeEditor's handle type. Called by:
 * FileEditor (registers its engine), panelRead (reads).
 * Invariants: an editor registers while it is mounted and removes only its own entry.
 */
import type { EditorHandle } from "../editor/CodeEditor";
import type { editorActions } from "../editor/actions";

const live = new Map<string, EditorHandle>();
const code = new Map<string, ReturnType<typeof editorActions>>();
const key = (sessionId: string, file: string) => `${sessionId}\u0000${file}`;

/** Registers the editor of `file` in `sessionId`; returns the function that removes it. */
export function registerLiveEditor(
  sessionId: string,
  file: string,
  handle: EditorHandle,
): () => void {
  live.set(key(sessionId, file), handle);
  return () => {
    if (live.get(key(sessionId, file)) === handle) live.delete(key(sessionId, file));
  };
}

/** The mounted editor of `file` in `sessionId`, if one is on screen. */
export function liveEditor(sessionId: string, file: string): EditorHandle | undefined {
  return live.get(key(sessionId, file));
}

/** Registers a comparison's CodeMirror operations without pretending it is the editable file's selection handle. */
export function registerCodeActions(
  sessionId: string,
  file: string,
  actions: ReturnType<typeof editorActions>,
): () => void {
  const id = key(sessionId, file);
  code.set(id, actions);
  return () => {
    if (code.get(id) === actions) code.delete(id);
  };
}

/** Operations of the mounted editor, including either side of a saved comparison. */
export function liveCodeActions(
  sessionId: string,
  file: string,
): ReturnType<typeof editorActions> | undefined {
  return code.get(key(sessionId, file)) ?? liveEditor(sessionId, file);
}
