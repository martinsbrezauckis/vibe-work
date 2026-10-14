/**
 * Plugs the File tab into the app: the tab body over the placeholder, the `ui/files` route, and the tab
 * button hidden until a file is opened (the tab appears when a click in the chat opens one).
 * Depends on: FileTab, filesRoutes, AppContext types. Called by: main.tsx, the UI test rig.
 * Invariants: without a core nothing is registered (the placeholder stays); the returned function undoes every
 * registration and hides the viewer.
 */
import type { App } from "../shell/AppContext";
import { FileTab } from "./FileTab";
import { registerFilesRoute } from "./filesRoutes";

/** Registers the File tab on `app`; returns the function that removes it. */
export function registerFilesTab(app: App): () => void {
  const files = app.files;
  if (files === null) return () => undefined;
  app.panelTabs.setHidden("file", true);
  const removers = [
    app.panelTabs.register("file", () => <FileTab files={files} />),
    registerFilesRoute(app, files),
  ];
  return () => {
    for (const remove of removers) remove();
    void files.layout(null);
    files.stop();
  };
}
