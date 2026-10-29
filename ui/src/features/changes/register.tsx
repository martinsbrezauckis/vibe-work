/**
 * Plugs the Changes tab into the app: creates the controller, registers the tab body over the
 * placeholder and the `ui/changes` route, loads the shown session's changes and git status, and offers the
 * tab's button only when that session has something to show (a folder without git and without changed files
 * shows no Changes tab and no git text).
 * Git status is re-read when the shown session starts or finishes a turn.
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

/** Registers the Changes tab and route on `app`; returns the function that removes them. */
export function registerChangesTab(app: App): () => void {
  const core = app.core;
  const changes = app.changes;
  if (core === null || changes === null) return () => undefined;
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
  const seenCommits: Record<string, object | undefined> = {};
  let shown: string | null = null;
  let shownStatus: string | null = null;
  let shownBar: import("../../protocol/core").GitStatusView | undefined;
  const sync = () => {
    const selected = app.shell.store.get().selected;
    const entry = selected === null ? undefined : core.store.get().sessions[selected];
    const status = entry === undefined ? null : statusType(entry.session);
    const finished = (shownStatus === "running" || shownStatus === "blocked") && status === "idle";
    const started = shownStatus === "idle" && status === "running";
    shownStatus = status;
    // A chosen stored session is read once it is open: until then the core has nothing to answer.
    const open = entry?.state === null ? null : selected;
    if (open !== shown) {
      shown = open;
      shownBar = undefined;
      if (open !== null) void changes.load(open);
      if (open !== null && git !== null) void git.load(open);
    } else if ((started || finished) && selected !== null && git !== null) {
      // Turn boundaries can change the commit guard without changing any files.
      void git.load(selected);
    }
    const view = selected === null ? undefined : changes.store.get().views[selected];
    const bar = selected === null || git === null ? undefined : git.store.get().status[selected];
    if (selected !== null && bar !== undefined && bar !== shownBar) {
      const previous = shownBar;
      shownBar = bar;
      if (bar.repo !== "." || previous?.repo !== ".")
        changes.selectRepository(selected, bar.repo ?? undefined);
    }
    const commit = selected === null ? undefined : git?.store.get().committed[selected];
    if (
      selected !== null &&
      commit !== undefined &&
      seenCommits[selected] !== commit &&
      commit.repo === bar?.repo
    ) {
      const id = selected;
      seenCommits[id] = commit;
      void changes.load(id).then(() => {
        changes.markSaved(id);
      });
    }
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
