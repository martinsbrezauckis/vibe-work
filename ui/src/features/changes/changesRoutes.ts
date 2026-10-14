/**
 * The `ui/changes` UI-control route: an external agent reads the Changes tab as it
 * is shown and moves around in it like a user (select a file, switch the layout, reload). It does not
 * keep or revert. Commands sent while a UI-control request is served go out with the Debug API's
 * origin (`state/coreStore.ts`), so actions could be added safely; they are left out because the Debug API
 * already keeps and reverts through `POST /commands` (`{"type": "changes",...}`, origin `debug-api`), with the
 * same core checks.
 * Depends on: state/uiControl, store.ts, model.ts, AppContext types. Called by: register.tsx. The Rust side
 * lists the route (`crates/vw-host/src/host.rs` SHELL_ROUTES).
 * Invariants: bodies are validated (`invalidParams`); `sessionId` is the shown session.
 */
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { commentAction, commentsState } from "./commentRoutes";
import type { LineComments } from "./lineComments";
import { fileNote, hasChanges, historyOfferShown, shownFile, turnText } from "./model";
import type { Changes } from "./store";

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

/** Registers `ui/changes`; returns the function that removes it. */
export function registerChangesRoute(
  app: App,
  changes: Changes,
  comments: LineComments,
): () => void {
  const state = () => {
    const sessionId = app.shell.store.get().selected;
    const ui = changes.store.get();
    const view = sessionId === null ? undefined : ui.views[sessionId];
    const file = view === undefined ? null : shownFile(view, ui.selected[view.sessionId] ?? null);
    return {
      sessionId,
      offered: hasChanges(view),
      mode: view?.mode ?? null,
      layout: ui.layout,
      files: (view?.files ?? []).map((f) => ({
        path: f.path,
        status: f.status,
        content: f.content,
        added: f.added,
        removed: f.removed,
        hunks: f.hunks.length,
        note: fileNote(f),
        turn: turnText(f.turn),
      })),
      shown:
        file === null
          ? null
          : {
              path: file.path,
              hunks: file.hunks.map((h) => ({
                id: h.id,
                oldStart: h.oldStart,
                newStart: h.newStart,
                lines: h.lines.length,
              })),
            },
      turns: (view?.turns ?? []).map((t) => ({
        number: t.number,
        files: t.files.map((f) => f.path),
      })),
      offerHistory:
        app.git === null
          ? (view?.offerHistory ?? false)
          : historyOfferShown(
              view,
              sessionId === null ? undefined : app.git.store.get().status[sessionId],
              sessionId === null ? undefined : app.git.store.get().selected[sessionId],
            ),
      confirm: ui.confirm?.kind ?? null,
      error: sessionId === null ? null : (ui.error[sessionId] ?? null),
      busy: sessionId === null ? false : ui.busy[sessionId] === true,
      comments: commentsState(comments, sessionId),
    };
  };
  return app.uiControl.register("changes", async (body) => {
    const { action, path, layout } = asObject(body);
    const sessionId = app.shell.store.get().selected;
    switch (action ?? "state") {
      case "state":
        return state();
      case "reload":
        if (sessionId === null) throw new UiControlError("invalidParams", "no session is shown");
        await changes.load(sessionId);
        return state();
      case "select":
        if (sessionId === null) throw new UiControlError("invalidParams", "no session is shown");
        if (typeof path !== "string" || path === "")
          throw new UiControlError("invalidParams", "path must be a non-empty string");
        changes.select(sessionId, path);
        return state();
      case "layout":
        if (layout !== "split" && layout !== "unified")
          throw new UiControlError("invalidParams", "layout must be split or unified");
        changes.setLayout(layout);
        return state();
      case "comment":
      case "removeComment":
      case "sendComments":
        if (sessionId === null) throw new UiControlError("invalidParams", "no session is shown");
        await comments.load(sessionId);
        await commentAction(String(action), asObject(body), { changes, comments, sessionId });
        return state();
      default:
        throw new UiControlError(
          "invalidParams",
          "action must be state, reload, select, layout, comment, removeComment or sendComments",
        );
    }
  });
}
