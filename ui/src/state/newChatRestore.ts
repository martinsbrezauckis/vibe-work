/**
 * What the New chat screen had open, kept across an app restart (`vw.newChatWorkspace` in localStorage): the
 * folder, the side panel's tabs, the open files' paths and the web tabs' addresses. Terminals are not kept: their
 * programs end with the app, and the Terminal tab opens one fresh shell in the folder when it shows again.
 * Depends on: shellState (tab kinds). Called by: newChatDraft.
 * Invariants: storage errors never throw (private mode, quota); anything malformed reads as nothing saved; only the
 * tab kinds that work before the chat are kept.
 */
import type { TabKind } from "./shellState";

const KEY = "vw.newChatWorkspace";

/** The tab kinds the New chat screen can show. */
const KINDS: readonly TabKind[] = ["browser", "git", "terminal", "file"];

/** The saved screen. */
export interface SavedNewChat {
  folder: string | null;
  /** The side panel's tabs in order (empty: the panel was closed). */
  tabs: readonly TabKind[];
  active: TabKind | null;
  /** Absolute paths of the open files. */
  files: readonly string[];
  /** Addresses of the web tabs. */
  pages: readonly string[];
}

const strings = (value: unknown): string[] =>
  Array.isArray(value) ? value.filter((v): v is string => typeof v === "string" && v !== "") : [];

/** The saved screen, or `null` when nothing (valid) is saved. */
export function loadNewChat(): SavedNewChat | null {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw === null) return null;
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return null;
    const value = parsed as Record<string, unknown>;
    const tabs = strings(value["tabs"]).filter((t): t is TabKind => KINDS.includes(t as TabKind));
    const active = typeof value["active"] === "string" ? (value["active"] as TabKind) : null;
    return {
      folder:
        typeof value["folder"] === "string" && value["folder"] !== "" ? value["folder"] : null,
      tabs,
      active: active !== null && tabs.includes(active) ? active : null,
      files: strings(value["files"]),
      pages: strings(value["pages"]),
    };
  } catch {
    return null;
  }
}

/** Saves `saved` (only when it differs from what is stored). */
export function saveNewChat(saved: SavedNewChat): void {
  try {
    const text = JSON.stringify({ ...saved, tabs: saved.tabs.filter((t) => KINDS.includes(t)) });
    if (localStorage.getItem(KEY) !== text) localStorage.setItem(KEY, text);
  } catch {
    // Not kept this time; the screen still works.
  }
}
