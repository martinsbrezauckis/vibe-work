/**
 * Files side-panel tab view: combines the file explorer (`FilesPanel`) and virtualized repository tree
 * (`GitBrowser`) and loads on demand when the Files tab renders so `virtua` and explorer UI stay out of first paint.
 * Depends on: Selector, GitBrowser, FilesPanel, adapter, routing, point, currentProject. Called by: git-browser/register.
 */
import { type ReactElement, useEffect } from "react";

import { Selector, type SelectorOption } from "../../components/Selector";
import { toCallError } from "../../protocol/client/types";
import type { GitBrowserSnapshot } from "../../protocol/core";
import { type Store, useStore } from "../../state/createStore";
import { term } from "../../vocabulary";
import { FilesPanel } from "../explorer/FilesPanel";
import type { PlaceStore } from "../explorer/panelModel";
import type { Explorer } from "../explorer/store";
import type { App } from "../shell/AppContext";
import { GitBrowser } from "./GitBrowser";
import { reposOf } from "./adapter";
import { currentProject } from "./currentProject";
import { pointGitFile } from "./point";
import { adapterAction, adapterFile } from "./routing";

/** View state shared between the git browser controller and its on-demand panel view. */
export type GitBrowserViewState = {
  projectId: string | null;
  snapshot: GitBrowserSnapshot | null;
  loading: boolean;
  error: string | null;
};

/** Renders the Files tab panel (`FilesPanel` + `GitBrowser`). */
export function GitBrowserView({
  app,
  view,
  watch,
  explorer,
  places,
}: {
  app: App;
  explorer: Explorer;
  places: PlaceStore;
  view: Store<GitBrowserViewState>;
  watch: (id: string | null) => Promise<void>;
}): ReactElement {
  const projects = app.projects;
  if (projects === null) throw new Error("Git browser requires saved projects.");
  const projectId = useStore(app.shell.store, () => currentProject(app));
  const groupData = useStore(projects.groups.store, (s) => s.data);
  const projectSelection = useStore(projects.store, (s) => s.selected);
  const state = useStore(view, (s) => s);
  const selectedSessionId = useStore(app.shell.store, (s) => s.selected);
  useEffect(() => {
    const id = currentProject(app);
    void watch(id);
  }, [app, watch, projectId, groupData, projectSelection]);
  const selectedId = currentProject(app);
  // The shared Selector, not a native select: its list follows the light and dark theme on every system (the
  // native list was drawn by Windows with a white background behind the dark theme's light text).
  const projectOptions: SelectorOption<string>[] = [
    { value: "", label: "Choose a project", testId: "git-browser.project-none" },
    ...(groupData?.projects ?? [])
      .filter((p) => p.mainFolder !== null)
      .map((p) => ({ value: p.id, label: p.name, testId: "git-browser.project-option" })),
  ];
  const chooser = app.shell.store.get().selected === null && (
    <div className="gb-project-choice">
      <span>{term("workNames", "projectTab").name}</span>
      <Selector<string>
        testId="git-browser.project-select"
        label={term("workNames", "projectTab").name}
        value={selectedId ?? ""}
        options={projectOptions}
        onChange={(value) => app.projects?.select(value || null)}
      />
    </div>
  );
  const snapshot = state.projectId === currentProject(app) ? state.snapshot : null;
  const act = (task: Promise<void>) => {
    void task.catch((error: unknown) => {
      view.set((s) => ({ ...s, error: toCallError(error).error.message }));
    });
  };
  const placeKey = selectedSessionId ?? selectedId ?? "new";
  return (
    <FilesPanel
      key={placeKey}
      app={app}
      explorer={explorer}
      snapshot={snapshot}
      places={places}
      placeKey={placeKey}
      legacy={
        <details className="fx-places">
          <summary data-testid="explorer.repositories">Repositories and chat copies</summary>
          {chooser}
          <GitBrowser
            inHistory
            repos={reposOf(snapshot)}
            note={snapshot?.truncated ? snapshot.note : null}
            selectedSessionId={selectedSessionId}
            loading={state.loading}
            error={state.error}
            onRetry={() => {
              const id = currentProject(app);
              if (id !== null) void watch(id);
            }}
            onAction={(target, action) => {
              act(adapterAction(app, target, action, "ui"));
            }}
            onFile={(target, path) => {
              act(adapterFile(app, target, path, "ui"));
            }}
            onPoint={(target, path) => {
              act(
                Promise.resolve().then(() => {
                  pointGitFile(app, target, path);
                }),
              );
            }}
          />
        </details>
      }
    />
  );
}
