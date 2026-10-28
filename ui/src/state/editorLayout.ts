/** Per-session editor layout reducers; used by shell actions and geometry. Switching the shared slot preserves widths. */
import { panelKey, type ShellState } from "./shellState";
export interface EditorLayout {
  split: boolean;
  slot: "chat" | "list" | "folded";
  width: number | null;
  chatFolded: boolean;
}
const DEFAULT: EditorLayout = { split: false, slot: "chat", width: null, chatFolded: false };
export function editorLayoutOf(state: ShellState): EditorLayout {
  return state.editorLayouts[panelKey(state)] ?? DEFAULT;
}
export function setEditorLayout(
  state: ShellState,
  patch: Partial<EditorLayout>,
  id = panelKey(state),
): ShellState {
  const before = state.editorLayouts[id] ?? DEFAULT;
  if (Object.entries(patch).every(([k, v]) => before[k as keyof EditorLayout] === v)) return state;
  return { ...state, editorLayouts: { ...state.editorLayouts, [id]: { ...before, ...patch } } };
}
export function swapEditorSlot(state: ShellState): ShellState {
  return setEditorLayout(state, {
    slot: editorLayoutOf(state).slot === "list" ? "chat" : "list",
    chatFolded: false,
  });
}
export function foldChat(state: ShellState): ShellState {
  return setEditorLayout(state, { slot: "folded", chatFolded: true });
}
export function unfoldChat(state: ShellState): ShellState {
  // Also ends a maximized panel: the chat strip is one of its folded columns.
  return { ...setEditorLayout(state, { slot: "chat", chatFolded: false }), maximized: false };
}
