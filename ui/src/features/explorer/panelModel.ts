/**
 * What the Files panel works on and how it opens a file: the folder it shows (the session's folder, the chosen
 * New chat folder or the project's main folder, unless a place was chosen), the Places choices, and the open
 * that routes a file to the session or the chosen-folder workspace. One model for FilesPanel and the
 * `ui/explorer` route, so a Debug API caller and a click reach the same folder and the same file view.
 * Depends on: state/createStore, sessionSummary, vocabulary, the explorer store, AppContext.
 * Called by: FilesPanel, explorerRoute, git-browser/register.
 * Invariants:
 * - A chosen place belongs to the panel it was chosen in (`key`: the session, else the project, else "new");
 *   switching session or project shows that panel's own folder again, as the remounted panel did before.
 * - Opening a file never starts a session: it selects the file's recorded session, or shows the chosen
 *   folder's workspace on the New chat view.
 */
import type { SelectorOption } from "../../components/Selector";
import type { GitBrowserSnapshot } from "../../protocol/core";
import { createStore } from "../../state/createStore";
import { summarize } from "../../state/sessionSummary";
import { TEXT } from "../../vocabulary";
import type { App } from "../shell/AppContext";
import type { Explorer } from "./store";

/** The place a person chose in one panel (`key`), or none. */
export interface ChosenPlace {
  key: string;
  place: string | null;
}

/** The chosen-place store, one per app (created with the Files tab). */
export function createPlaceStore() {
  return createStore<ChosenPlace>({ key: "", place: null });
}
export type PlaceStore = ReturnType<typeof createPlaceStore>;

/** The panel's folder, its Places and the session it opens files in. */
export interface PanelContext {
  /** The selected session's folder, when a session is selected. */
  sessionFolder: string | null;
  /** The folder before any chosen place. */
  base: string | null;
  /** The folder the panel shows. */
  folder: string | null;
  places: SelectorOption<string>[];
  /** A session is selected (Commit and Revert all live in its Changes tab then). */
  session: boolean;
}

/** The last part of `path` (either separator), or the path itself for a root. */
function folderName(path: string): string {
  return (
    path
      .split(/[\\/]/)
      .filter((part) => part !== "")
      .pop() ?? path
  );
}

/** Computes the panel for `key` from the current app state (reads only). */
export function panelContext(
  app: App,
  snapshot: GitBrowserSnapshot | null,
  chosen: ChosenPlace,
  key: string,
): PanelContext {
  const { selected, newFolder } = app.shell.store.get();
  const entry = selected ? app.core?.store.get().sessions[selected] : undefined;
  const sessionFolder = entry ? summarize(entry, {}).cwd : null;
  const base = sessionFolder ?? newFolder ?? snapshot?.folder ?? null;
  const folder = (chosen.key === key ? chosen.place : null) ?? base;
  const places: SelectorOption<string>[] = [];
  const add = (value: string, label: string, disabled = false) => {
    if (!places.some((o) => o.value === value)) places.push({ value, label, disabled });
  };
  // Without a session the folder is called by its own name (the path is shown beside it).
  if (base) add(base, sessionFolder ? `${TEXT.thisSession}'s folder` : folderName(base));
  if (snapshot?.folder) add(snapshot.folder, "Project's main folder");
  for (const node of snapshot?.nodes ?? [])
    add(
      node.path,
      node.kind === "copy"
        ? `Session copy: ${node.name}${node.branch ? ` · ${node.branch}` : ""}`
        : node.name,
      node.outsideProject && !node.sessionId,
    );
  if (folder) add(folder, folder);
  return { sessionFolder, base, folder, places, session: selected !== null };
}

/**
 * Where a file of the panel's folder opens: the selected session when it works in that folder, the session of
 * that Places entry, else the folder's New chat workspace (`files-workspace:<folder>`); null without a folder.
 */
export function panelOwner(
  app: App,
  context: PanelContext,
  snapshot: GitBrowserSnapshot | null,
): string | null {
  const folder = context.folder;
  if (!folder) return null;
  const selected = app.shell.store.get().selected;
  const placeSession = snapshot?.nodes.find((node) => node.path === folder)?.sessionId;
  return selected !== null && context.sessionFolder === folder
    ? selected
    : (placeSession ?? `files-workspace:${folder}`);
}

/** The words shown when a file cannot be opened. */
export const OPEN_FAILED = "This file could not be opened. Refresh the folder and choose it again.";

/**
 * Opens `path` (relative to the panel's folder) at `line` in the File tab, like a click on a tree row, search
 * result or commit file. Answers whether it opened; side effects: selects the session (or the New chat view
 * with the chosen folder), shows the File tab. `origin` picks the file actions: the window's own, or the Debug
 * API's (every read carries that origin, as `ui/files` does).
 */
export async function openInPanel(
  app: App,
  explorer: Explorer,
  context: PanelContext,
  snapshot: GitBrowserSnapshot | null,
  path: string,
  line: number | undefined,
  origin: "ui" | "debug-api",
): Promise<boolean> {
  const folder = context.folder;
  if (!folder || !app.files) return false;
  const sessionKey = panelOwner(app, context, snapshot) ?? `files-workspace:${folder}`;
  const actions = origin === "ui" ? app.files.actions : app.files.debugApi;
  const shown = await actions.openText(sessionKey, `${folder}/${path}`, line);
  if (!shown || explorer.store.get().folder !== folder) return false;
  if (sessionKey.startsWith("files-workspace:")) {
    app.shell.actions.setNewFolder(folder, "session");
    app.shell.actions.selectSession(null);
  } else app.shell.actions.selectSession(sessionKey);
  app.panelTabs.setHidden("file", false);
  app.shell.actions.openTab("file");
  return true;
}
