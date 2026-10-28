/**
 * The `ui/worktrees` UI-control route: an external agent reads the worktree UI as it
 * is shown (the explicit removal inspection, the startup list of copies from deleted sessions)
 * and chooses a folder for the new-session view (`folder`), the step a user does with Choose folder.
 * (the Debug API acts as the user): `folder` picks the folder through the core's `pickFolder` (it
 * then counts as picked for this run, like a native pick) and fills the view, marked as chosen by the Debug API
 * (a label: the view says so); `state` returns what the window shows, names and paths included. Starting,
 * and removing go through `POST /commands` (`{"type": "worktrees",...}`) or clicks.
 * Depends on: state/uiControl, AppContext types. Called by: shell/useShellWiring. The Rust side lists the route
 * (`crates/vw-host/src/host.rs`).
 * Invariants: bodies are validated (`invalidParams`); `folder` sends one command (`pickFolder`, Debug API
 * origin) and fills the view only when the core accepted the folder.
 */
import { toCallError } from "../../protocol/client/types";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

/** Registers `ui/worktrees`; returns the function that removes it. */
export function registerWorktreeRoutes(app: App): () => void {
  const state = () => {
    const ui = app.worktrees?.store.get();
    const shell = app.shell.store.get();
    return {
      newFolder: shell.newFolder,
      newFolderBy: shell.newFolderBy,
      end:
        ui?.end === null || ui?.end === undefined
          ? null
          : {
              sessionId: ui.end.sessionId,
              busy: ui.end.busy,
              inspection: ui.end.inspection,
            },
      orphans: ui?.orphans ?? [],
    };
  };
  const pick = async (path: string) => {
    if (app.core === null) throw new UiControlError("serverUnavailable", "the core is not running");
    try {
      // The host runs exactly this command as the Debug API's under the request's grant (`ui_grants.rs`).
      await app.core.run({ type: "pickFolder", path });
    } catch (thrown) {
      const { error } = toCallError(thrown);
      throw new UiControlError(error.code, error.message);
    }
    app.shell.actions.selectSession(null);
    app.shell.actions.setNewFolder(path, "debug-api");
    return state();
  };
  return app.uiControl.register("worktrees", (body) => {
    const { action, path } = asObject(body);
    switch (action ?? "state") {
      case "state":
        return (app.worktrees?.loadOrphans() ?? Promise.resolve()).then(state);
      case "folder":
        if (typeof path !== "string" || path === "")
          throw new UiControlError("invalidParams", "path must be a non-empty string");
        return pick(path);
      default:
        throw new UiControlError("invalidParams", "unknown action");
    }
  });
}
