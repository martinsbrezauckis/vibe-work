/** Editor workspace shortcuts share the visible controls' actions. Registered with the File tab; search stays in the palette. */
import type { App } from "../shell/AppContext";
import { panelOf } from "../../state/shellState";
import { editorLayoutOf } from "../../state/editorLayout";
export function registerEditorKeys(app: App): () => void {
  const files = app.files;
  if (!files) return () => undefined;
  const split = () => {
    const state = app.shell.store.get();
    const id = state.selected;
    if (
      id === null ||
      panelOf(state).active !== "file" ||
      state.settings.open ||
      state.overlay !== null
    )
      return;
    const view = files.workspace.of(id);
    const doc = view.documents[view.active ?? ""];
    if (doc) void files.workspace.openToSide(id, doc.shown);
  };
  const removers = [
    app.shortcuts.register({
      id: "files.split",
      keys: "Mod+\\",
      description: "Open to the side",
      run: split,
    }),
    app.shortcuts.register({
      id: "files.split-enter",
      keys: "Mod+Enter",
      description: "Open to the side",
      run: split,
    }),
    app.shortcuts.register({
      id: "files.close-tab",
      keys: "Mod+W",
      description: "Close the file tab",
      run: () => {
        const s = app.shell.store.get();
        const id = s.selected;
        if (id !== null && panelOf(s).active === "file" && !s.settings.open && s.overlay === null) {
          const f = files.workspace.of(id).active;
          if (f) files.workspace.requestClose(id, f);
        }
      },
    }),
    app.shortcuts.register({
      id: "files.quick-open",
      keys: "Mod+P",
      description: "Find a file",
      run: () => {
        app.shell.actions.setOverlay("palette", "", true);
      },
    }),
    app.shortcuts.register({
      id: "chat.fold",
      keys: "Mod+Alt+B",
      description: "Fold chat",
      run: () => {
        if (editorLayoutOf(app.shell.store.get()).chatFolded) app.shell.actions.unfoldChat();
        else app.shell.actions.foldChat();
      },
    }),
  ];
  return () => {
    removers.forEach((remove) => {
      remove();
    });
  };
}
