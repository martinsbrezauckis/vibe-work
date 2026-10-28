/**
 * How the rest of the app reaches the Files tab of one app instance: Reveal in Files, History of this file and
 * the commit reads of History's menus. The Files tab binds its explorer when it registers.
 * Depends on: the explorer store, AppContext types. Called by: git-browser/register (binds) and the file menus.
 */
import type { App } from "../shell/AppContext";
import type { Explorer } from "./store";

/** What the Files tab offers to the menus. */
export interface FilesBridge {
  explorer: Explorer;
  /** Shows `folder` in the Files tab (the chosen place) when it shows another folder now. */
  showFolder: (folder: string) => void;
}

const bridges = new WeakMap<App, FilesBridge>();

/** Binds the Files tab of `app`; returns the function that unbinds it. */
export function bindFiles(app: App, bridge: FilesBridge): () => void {
  bridges.set(app, bridge);
  return () => {
    if (bridges.get(app) === bridge) bridges.delete(app);
  };
}

/** The Files tab of `app`, or null before it registered (no core). */
export function filesOf(app: App): FilesBridge | null {
  return bridges.get(app) ?? null;
}
