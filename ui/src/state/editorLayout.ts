/** Per-session editor layout reducers; used by shell actions and geometry. Switching the shared slot preserves widths. */
import { panelKey, type ShellState } from "./shellState";
/** One session's editor split: which column shares the screen with the panel and its dragged width. */
export interface EditorLayout {
  split: boolean;
  slot: "chat" | "list" | "folded";
  width: number | null;
  chatFolded: boolean;
}
const DEFAULT: EditorLayout = { split: false, slot: "chat", width: null, chatFolded: false };
/** The editor layout of the current view, or the default when none is saved. */
export function editorLayoutOf(state: ShellState): EditorLayout {
  return state.editorLayouts[panelKey(state)] ?? DEFAULT;
}
/** Applies `patch` to a session's editor layout (the current view by default); returns `state` when nothing changes. */
export function setEditorLayout(
  state: ShellState,
  patch: Partial<EditorLayout>,
  id = panelKey(state),
): ShellState {
  const before = state.editorLayouts[id] ?? DEFAULT;
  if (Object.entries(patch).every(([k, v]) => before[k as keyof EditorLayout] === v)) return state;
  return { ...state, editorLayouts: { ...state.editorLayouts, [id]: { ...before, ...patch } } };
}
/** Switches the shared slot between the chat and the list, and unfolds the chat. */
export function swapEditorSlot(state: ShellState): ShellState {
  return setEditorLayout(state, {
    slot: editorLayoutOf(state).slot === "list" ? "chat" : "list",
    chatFolded: false,
  });
}
/** Folds the chat into a strip in the shared slot. */
export function foldChat(state: ShellState): ShellState {
  return setEditorLayout(state, { slot: "folded", chatFolded: true });
}
/** Shows the chat in the shared slot and ends a maximized panel. */
export function unfoldChat(state: ShellState): ShellState {
  // Also ends a maximized panel: the chat strip is one of its folded columns.
  return { ...setEditorLayout(state, { slot: "chat", chatFolded: false }), maximized: false };
}
