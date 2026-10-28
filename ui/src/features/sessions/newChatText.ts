/**
 * The plain-word texts of the New chat screen's open things: the status line under the Files · Terminal · Browser
 * row, the Close all question, and the session list's badge. Pure; no state.
 * Depends on: state/newChatDraft (the counts' type). Called by: NewChatWorkspace, SessionList, tests.
 */
import type { NewChatOpen } from "../../state/newChatDraft";

/** `n` with the word, plural from two on ("1 terminal", "2 terminals"). */
function count(n: number, word: string): string {
  return `${String(n)} ${word}${n === 1 ? "" : "s"}`;
}

/** "Open here: 1 terminal · Browser, 2 pages · 3 files" (only the parts that apply), or `null` when nothing is open. */
export function openSummary(open: NewChatOpen, files: number): string | null {
  const parts = [
    open.terminals > 0 ? count(open.terminals, "terminal") : null,
    open.pages > 0 ? `Browser, ${count(open.pages, "page")}` : null,
    files > 0 ? count(files, "file") : null,
  ].filter((part): part is string => part !== null);
  return parts.length === 0 ? null : `Open here: ${parts.join(" · ")}`;
}

/** The Close all question's body: what stops and closes, and what stays. */
export function closeAllBody(open: NewChatOpen, files: number): string {
  return [
    open.terminals > 0
      ? `${count(open.terminals, "terminal")} ${open.terminals === 1 ? "stops, with any program running in it." : "stop, with any program running in them."}`
      : null,
    open.pages > 0
      ? `${count(open.pages, "browser page")} ${open.pages === 1 ? "closes" : "close"}.`
      : null,
    files > 0 ? "Open files close; unsaved edits are kept until you decide." : null,
    "Your text and folder stay.",
  ]
    .filter((part): part is string => part !== null)
    .join(" ");
}

/** The badge on the list's New chat button ("2 open"), or `null` when no terminal or page is open there. */
export function badgeText(open: NewChatOpen): string | null {
  const n = open.terminals + open.pages;
  return n === 0 ? null : `${String(n)} open`;
}

/** The badge's tooltip. */
export function badgeTitle(open: NewChatOpen, keys: string): string {
  return `Back to your new chat: ${count(open.terminals, "terminal")} and ${count(open.pages, "page")} are open there (${keys})`;
}
