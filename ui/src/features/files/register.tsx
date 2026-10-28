/**
 * Plugs the File tab into the app: the tab body over the placeholder, the `ui/files` and `ui/editors` routes, the tab
 * button hidden until a file is opened (the tab appears when a click in the chat opens one), and the composer's
 * step that saves edited open files before a message goes out, and the answer to the agent's `panel_read`.
 * Depends on: FileTab, filesRoutes, AppContext types. Called by: main.tsx, the UI test rig.
 * Invariants: without a core nothing is registered (the placeholder stays); the returned function undoes every
 * registration and hides the viewer.
 */
import { registerEditorKeys } from "./editorKeys";
import type { App } from "../shell/AppContext";
import { FileTab } from "./FileTab";
import { registerFilesRoute } from "./filesRoutes";
import { registerEditorsRoute } from "./editorsRoute";
import { saveOpenFiles } from "./saveBeforeSend";
import { registerPanelRead } from "./panelRead";

/** Registers the File tab on `app`; returns the function that removes it. */
export function registerFilesTab(app: App): () => void {
  const files = app.files;
  if (files === null) return () => undefined;
  app.panelTabs.setHidden("file", true);
  // How many files each session had open at the last update: the tab closes when the last one closes, not on every
  // update while none is open (an open selects the tab before its file arrives).
  const counts = new Map<string, number>();
  const sync = () => {
    const before = new Map(counts);
    for (const [id, view] of Object.entries(files.workspace.store.get())) {
      counts.set(id, view.order.length);
      if (view.order.length > 0 && app.shell.store.get().panels[id]?.tabs.includes("file"))
        app.shell.actions.pinTab("git", id);
      app.shell.actions.setEditorLayout(
        {
          split: view.split !== null,
          ...(view.split === null && app.shell.store.get().editorLayouts[id]?.split
            ? { slot: "chat", chatFolded: false }
            : {}),
        },
        id,
      );
    }
    const id = app.shell.store.get().selected;
    if (
      id !== null &&
      files.workspace.of(id).order.length === 0 &&
      (before.get(id) ?? 0) > 0 &&
      app.shell.store.get().panels[id]?.tabs.includes("file")
    )
      app.shell.actions.closeTab("file");
  };
  // a message saves the session's edited open files first (Settings › General).
  app.composer.setBeforeSend((id, origin) => saveOpenFiles(app, id, origin));
  const removers = [
    () => {
      app.composer.setBeforeSend(null);
    },
    registerEditorKeys(app),
    registerPanelRead(app),
    files.workspace.store.subscribe(sync),
    app.panelTabs.register("file", () => <FileTab files={files} />),
    registerFilesRoute(app, files),
    registerEditorsRoute(app, files),
  ];
  return () => {
    for (const remove of removers) remove();
    void files.layout(null);
    files.stop();
  };
}
