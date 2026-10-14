/**
 * Plugs the Changes tab into the app: creates the controller, registers the tab body over the
 * placeholder and the `ui/changes` route, loads the shown session's changes and git status, and offers the
 * tab's button only when that session has something to show (a folder without git and without changed files
 * shows no Changes tab and no git text).
 * Git status is re-read when the shown session finishes a turn.
 * Depends on: store.ts, ChangesTab, changesRoutes, model.ts, AppContext types. Called by: main.tsx (after
 * the placeholders), the UI test rig.
 * Invariants: without a core nothing is registered (the placeholder stays); the returned function undoes
 * every registration and stops following events.
 */
import { statusType } from "../../state/sessionSummary";
import { registerGitRoute } from "../git/gitRoutes";
import { barShown } from "../git/model";
import type { App } from "../shell/AppContext";
import { ChangesTab } from "./ChangesTab";
import { registerChangesRoute } from "./changesRoutes";
import { LineCommentsProvider } from "./LineCommentsContext";
import { createLineComments } from "./lineComments";
import { hasChanges } from "./model";
import { createChanges } from "./store";

/** Registers the Changes tab and route on `app`; returns the function that removes them. */
export function registerChangesTab(app: App): () => void {
  const core = app.core;
  if (core === null) return () => undefined;
  const changes = createChanges(core);
  const comments = createLineComments(app, core, changes);
  const removeBody = app.panelTabs.register("changes", () => (
    <LineCommentsProvider value={comments}>
      <ChangesTab changes={changes} />
    </LineCommentsProvider>
  ));
  const removeRoute = registerChangesRoute(app, changes, comments);
  // the branch bar's route, and the bar keeps the tab offered after everything is committed.
  const git = app.git;
  const removeGitRoute = git === null ? () => undefined : registerGitRoute(app, git);
  let shown: string | null = null;
  let shownStatus: string | null = null;
  const sync = () => {
    const selected = app.shell.store.get().selected;
    const entry = selected === null ? undefined : core.store.get().sessions[selected];
    const status = entry === undefined ? null : statusType(entry.session);
    const finished = (shownStatus === "running" || shownStatus === "blocked") && status === "idle";
    shownStatus = status;
    if (selected !== shown) {
      shown = selected;
      if (selected !== null) void changes.load(selected);
      if (selected !== null && git !== null) void git.load(selected);
    } else if (finished && selected !== null && git !== null) {
      // File changes can fetch `running: true`; becoming idle need not change any files.
      void git.load(selected);
    }
    const view = selected === null ? undefined : changes.store.get().views[selected];
    const bar = selected === null || git === null ? undefined : git.store.get().status[selected];
    app.panelTabs.setHidden("changes", !hasChanges(view) && !barShown(bar));
  };
  const stopShell = app.shell.store.subscribe(sync);
  const stopCore = core.store.subscribe(sync);
  const stopChanges = changes.store.subscribe(sync);
  const stopGit = git === null ? () => undefined : git.store.subscribe(sync);
  sync();
  return () => {
    stopChanges();
    stopGit();
    stopShell();
    stopCore();
    removeGitRoute();
    removeRoute();
    removeBody();
    comments.stop();
    changes.stop();
    app.panelTabs.setHidden("changes", false);
  };
}
