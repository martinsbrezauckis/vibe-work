/**
 * The `ui/search` UI-control route: every search action a user has, through the same
 * functions the keyboard and clicks use, with the Debug API origin:
 * - `palette {query}`: opens the global search with the words typed (Ctrl K, then typing); `results` reads the rows as
 *   the palette shows them now (kind, title, path, opens); `run {id}` runs one like Enter;
 * - `find {query?, sessionId?}`: Ctrl F in a conversation (the find bar, with "n of m"); `next` / `previous` step;
 *   `findState` reads the bar; `findClose` closes it;
 * - `fileFind {query?}`: Ctrl F in the File tab; `fileNext` / `filePrevious`; `fileFindClose`.
 * The palette publishes its rows here on every render ([`publishResults`]), so `results` is what is on screen.
 * Depends on: AppContext, state/findStore, uiControl, paletteTypes. Called by: register (search feature).
 * Invariants: bodies are validated (`invalidParams`); `run` of an id not on screen is `notFound`.
 */
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import type { Result } from "./paletteTypes";

/** The palette's rows as last rendered, per app. */
const shown = new WeakMap<App, { query: string; rows: readonly Result[] }>();

/** Called by the palette on every render. */
export function publishResults(app: App, query: string, rows: readonly Result[]): void {
  shown.set(app, { query, rows });
}

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function optionalText(value: unknown, name: string): string | undefined {
  if (value === undefined) return undefined;
  if (typeof value === "string") return value;
  throw new UiControlError("invalidParams", `${name} must be a string`);
}

/** The bars as JSON. */
function findState(app: App): unknown {
  const { conversation, file } = app.find.store.get();
  return {
    conversation:
      conversation === null
        ? null
        : { sessionId: conversation.sessionId, query: conversation.query, at: conversation.at },
    file: file === null ? null : { sessionId: file.sessionId, query: file.query, at: file.at },
    counts: { ...lastCounts.get(app) },
  };
}

/** Match counts the bars report (`{conversation, file}`), for `findState`. */
const lastCounts = new WeakMap<App, { conversation?: number; file?: number }>();

/** Called by a find bar when its match count changes. */
export function publishCount(app: App, scope: "conversation" | "file", count: number): void {
  lastCounts.set(app, { ...lastCounts.get(app), [scope]: count });
}

/** Registers the route; returns the function that removes it. */
export function registerSearchRoutes(app: App): () => void {
  return app.uiControl.register("search", (body) => {
    const input = asObject(body);
    const action = input["action"] ?? "results";
    const selected = () => {
      const id = optionalText(input["sessionId"], "sessionId") ?? app.shell.store.get().selected;
      if (id === null) throw new UiControlError("invalidParams", "no session is open");
      return id;
    };
    const count = (scope: "conversation" | "file") => lastCounts.get(app)?.[scope] ?? 0;
    switch (action) {
      case "palette":
        app.shell.actions.setOverlay("palette", optionalText(input["query"], "query") ?? "");
        return { open: true };
      case "results": {
        const now = shown.get(app);
        const open = app.shell.store.get().overlay === "palette";
        return {
          open,
          query: open ? (now?.query ?? "") : "",
          rows: open
            ? (now?.rows ?? []).map(({ id, kind, title, path, opens }) => ({
                id,
                kind,
                title,
                path,
                opens,
              }))
            : [],
        };
      }
      case "run": {
        const id = optionalText(input["id"], "id");
        const row = shown.get(app)?.rows.find((r) => r.id === id);
        if (row === undefined || app.shell.store.get().overlay !== "palette")
          throw new UiControlError("notFound", "no result with that id is on screen");
        app.shell.actions.setOverlay(null);
        row.run();
        return { ran: row.id, kind: row.kind };
      }
      case "find":
        app.find.actions.open(selected(), optionalText(input["query"], "query"));
        return findState(app);
      case "next":
      case "previous":
        app.find.actions.step("conversation", action === "next" ? 1 : -1, count("conversation"));
        return findState(app);
      case "findClose":
        app.find.actions.close("conversation");
        return findState(app);
      case "fileFind":
        app.find.actions.openFile(selected(), optionalText(input["query"], "query"));
        return findState(app);
      case "fileNext":
      case "filePrevious":
        app.find.actions.step("file", action === "fileNext" ? 1 : -1, count("file"));
        return findState(app);
      case "fileFindClose":
        app.find.actions.close("file");
        return findState(app);
      case "findState":
        return findState(app);
      default:
        throw new UiControlError(
          "invalidParams",
          "action must be palette, results, run, find, next, previous, findClose, fileFind, fileNext, filePrevious, fileFindClose or findState",
        );
    }
  });
}
