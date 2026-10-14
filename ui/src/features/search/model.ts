/**
 * Search results that open as what they are: a message opens its conversation at the match with the find bar,
 * a file opens the File tab at the line. Pure builders plus the two open actions.
 * Depends on: AppContext types, protocol types, state/findStore. Called by: the command palette, the `ui/search`
 * route.
 * Invariants: every result carries a path to the part that holds the match (`folder › title › Agent reply, turn 4`,
 * `folder › src/config.toml:8`); opening never invents a location (a file opens only through the core's resolve,
 * in a session whose folder holds it).
 */
import type { FileHit, MessageHit } from "../../protocol/core";
import type { App } from "../shell/AppContext";

/** The last folder name of a path (`/work/shop` -> `shop`, `C:\work\shop` -> `shop`). */
export function folderName(path: string | null): string {
  if (path === null || path === "") return "";
  const parts = path.split(/[\\/]+/).filter((p) => p !== "");
  return parts[parts.length - 1] ?? path;
}

/** The path shown under a message result. */
export function messagePath(hit: MessageHit): string[] {
  const who = hit.role === "user" ? "Your message" : "Agent reply";
  return [
    folderName(hit.cwd),
    hit.title ?? "Untitled session",
    `${who}, turn ${String(hit.turn)}`,
  ].filter((p) => p !== "");
}

/** The path shown under a file result. */
export function filePath(hit: FileHit): string[] {
  const where = hit.line === null ? hit.path : `${hit.path}:${String(hit.line)}`;
  return [folderName(hit.folder), where];
}

/** Opens a message hit: its conversation, scrolled to the match, the find bar open with the words. */
export async function openMessage(app: App, hit: MessageHit, query: string): Promise<void> {
  app.shell.actions.closeSettings();
  await app.sessions?.open(hit.sessionId);
  app.find.actions.open(hit.sessionId, query, hit.entryId);
}

/**
 * Opens a file hit: the session that works in its folder (the open one when it does), then the File tab at the
 * line (source view, so the line is visible). Returns false when no session can show it.
 */
export async function openFile(app: App, hit: FileHit): Promise<boolean> {
  const selected = app.shell.store.get().selected;
  const sessionId = hit.sessionId ?? selected;
  if (sessionId === null || app.files === null) return false;
  app.shell.actions.closeSettings();
  if (selected !== sessionId) await app.sessions?.open(sessionId);
  // The folder may differ from the session's own (a workspace root): an absolute path resolves either way.
  const sep = hit.folder.includes("\\") && !hit.folder.includes("/") ? "\\" : "/";
  const text = `${hit.folder}${hit.folder.endsWith(sep) ? "" : sep}${hit.path}`;
  const opened = await app.files.actions.openText(sessionId, text, hit.line);
  if (!opened) return false;
  app.shell.actions.openTab("file");
  if (hit.line !== null) await app.files.actions.setMode(sessionId, "source");
  return true;
}
