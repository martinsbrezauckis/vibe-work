/**
 * What a key press in the composer means: idle, Enter sends and Shift+Enter adds a line; while a turn runs,
 * Enter steers it and Tab queues the message for after it; with the `/` menu open, arrows move, Enter or
 * Tab picks, Escape closes.
 * Depends on: nothing. Called by: the composer view.
 * Invariants: pure; an IME composition (`isComposing`) never sends; modified Enter (Ctrl/Alt/Meta) is
 * left to the browser and the shortcuts layer.
 */

/** The parts of a keyboard event this needs. */
export interface KeyInfo {
  key: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  isComposing: boolean;
}

/** The composer's situation when the key arrives. */
export interface KeyContext {
  running: boolean;
  slashOpen: boolean;
  /** The text box holds something to send (text, chips or images); Tab queues only then. */
  hasContent: boolean;
}

/** The action for a key, or `null` for the browser's default. */
export type KeyAction =
  "send" | "steer" | "queue" | "slashNext" | "slashPrev" | "slashPick" | "slashClose" | null;

/** Maps a key press to the composer action. */
export function keyAction(key: KeyInfo, ctx: KeyContext): KeyAction {
  if (key.isComposing) return null;
  const plain = !key.ctrlKey && !key.altKey && !key.metaKey;
  if (ctx.slashOpen) {
    if (key.key === "ArrowDown") return "slashNext";
    if (key.key === "ArrowUp") return "slashPrev";
    if ((key.key === "Enter" || key.key === "Tab") && plain && !key.shiftKey) return "slashPick";
    if (key.key === "Escape") return "slashClose";
  }
  // Enter never inserts a line break (Shift+Enter does); with nothing to send it does nothing.
  if (key.key === "Enter" && plain && !key.shiftKey) return ctx.running ? "steer" : "send";
  if (key.key === "Tab" && plain && !key.shiftKey && ctx.running && ctx.hasContent) return "queue";
  return null;
}
