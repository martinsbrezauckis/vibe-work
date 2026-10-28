/**
 * Connects Files to the current session folder, chosen workspace or project, the existing file opener and the
 * tab's right-click menus (`explorerMenus.ts`). The
 * folder, Places and opening come from `panelModel.ts` and the folder actions from `workspaceModel.ts`, the same
 * models the `ui/explorer` route uses. Called by git-browser/register (the Files tab body).
 */
import { useEffect, useState, type ReactNode } from "react";
import { useStore } from "../../state/createStore";
import { ExplorerView } from "./ExplorerView";
import { WorkspaceActions } from "./WorkspaceActions";
import type { Explorer } from "./store";
import type { App } from "../shell/AppContext";
import type { GitBrowserSnapshot } from "../../protocol/core";
import { OPEN_FAILED, openInPanel, panelContext, panelOwner, type PlaceStore } from "./panelModel";
import type { ExplorerMenuDeps } from "./explorerMenus";
import { compareWithCommit } from "../files/fileMenu";
import { workspaceModelOf } from "./workspaceModel";

export function FilesPanel({
  app,
  explorer,
  snapshot,
  legacy,
  places,
  placeKey,
}: {
  app: App;
  explorer: Explorer;
  snapshot: GitBrowserSnapshot | null;
  legacy: ReactNode;
  /** The chosen place (shared with the `ui/explorer` route). */
  places: PlaceStore;
  /** This panel's key: the selected session, else the project, else "new". */
  placeKey: string;
}) {
  // Re-render on what panelContext reads: the selection, the New chat folder and the sessions' folders.
  useStore(app.shell.store, (s) => s.selected);
  useStore(app.shell.store, (s) => s.newFolder);
  const core = app.core;
  if (core === null) throw new Error("Files requires a core connection.");
  useStore(core.store, (s) => s.sessions);
  const chosen = useStore(places, (s) => s);
  const context = panelContext(app, snapshot, chosen, placeKey);
  const { folder } = context;
  const workspace = workspaceModelOf(app, explorer);
  const init = useStore(workspace.store, (s) => s.init);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (explorer.store.get().folder === folder) explorer.refresh();
    else explorer.choose(folder);
  }, [explorer, folder]);
  useEffect(() => {
    let last: unknown;
    return app.git?.store.subscribe(() => {
      const committed = app.git?.store.get().committed[`files-workspace:${folder ?? ""}`];
      if (committed && committed !== last) {
        last = committed;
        explorer.refresh();
      }
    });
  }, [app.git, explorer, folder]);
  const open = (path: string, line?: number) => {
    void openInPanel(app, explorer, context, snapshot, path, line, "ui")
      .then((shown) => {
        if (!shown && folder && app.files) setError(OPEN_FAILED);
      })
      .catch(() => {
        setError(OPEN_FAILED);
      });
  };
  const owner = panelOwner(app, context, snapshot);
  const onReview = () => {
    app.panelTabs.setHidden("changes", false);
    app.shell.actions.openTab("changes");
  };
  const menuDeps: ExplorerMenuDeps | null =
    owner === null || folder === null
      ? null
      : { app, explorer, owner, folder, session: context.session, workspace, onReview };
  return (
    <>
      <ExplorerView
        menuDeps={menuDeps}
        onCompare={(path, version) => {
          if (owner !== null && folder !== null)
            void compareWithCommit(app, { owner, folder, path }, version.id);
        }}
        explorer={explorer}
        places={context.places}
        session={context.session}
        onPlace={(value) => {
          places.set({ key: placeKey, place: value });
          workspace.closeInit();
        }}
        onOpen={open}
        onChoose={() => void app.sessions?.pickFolder()}
        onReview={onReview}
        onInit={() => {
          workspace.openInit();
        }}
        legacy={legacy}
        workspaceActions={
          <WorkspaceActions
            key={folder}
            app={app}
            explorer={explorer}
            init={false}
            onInitClose={() => undefined}
          />
        }
      />
      {init && (
        <div className="fx-init">
          <WorkspaceActions
            key={folder}
            app={app}
            explorer={explorer}
            init
            onInitClose={() => undefined}
          />
        </div>
      )}
      {error && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
    </>
  );
}
