/**
 * The `sessions` and `notifications` UI-control routes, registered next to the shell's
 * own (the Rust side lists the same names, `crates/vw-host/src/host.rs`). Each action goes through
 * the same functions a click uses, so a route can never do something the UI cannot, and every
 * answer is the list model as rendered, which is what the end-to-end test asserts on.
 * Depends on: AppContext types, listModel, state/*. Called by: useSessionWiring.
 * `temporaryChat {on}` sets the new-chat view's "Temporary chat" switch; the list answer carries `temporary`
 * (ids) and `newChatTemporary`.
 * Stuck sessions: `retryReopen`, `cancelReopen`, `continueNew` act like the row menu's items.
 * project actions (each through the same `groups.*` function a click uses; a route call carries the Debug
 * API origin and acts directly, as the user would): `projectCreate {name}`, `projectRename
 * {projectId, name}`, `projectDelete {projectId}` (the window's own confirmation is the user's, not asked here), `projectMove
 * {sessionIds, projectId|null}`, `projectPlace {projectId, before|null}`, `projectStart {projectId}`, `projectFolder
 * {projectId, folder}` (a folder a session works in, or `null` to clear; `folder` is required because the native
 * picker cannot be answered remotely and would hold the window's request queue), `projectsRefresh` (reconcile with Vibe's list), `toggleGroup {key}`.
 * `projectStart {projectId, folder?}` of a project without a main folder names the folder to run in (`folder`, a path;
 * the picker cannot be answered remotely) or is `invalidParams` (or set the project's folder first).
 * Invariants: bodies are validated (`invalidParams` on a bad one); `notifications focus` changes
 * only whether a notification is shown; delete is the click's flow, without the native confirmation for the
 * Debug API (it acts as the user).
 */
import { NOTIFY_LEVELS } from "../../state/osNotify";
import { startFolderFor } from "../../state/projectStartFolder";
import { summarize } from "../../state/sessionSummary";
import { UiControlError } from "../../state/uiControl";
import { waitingSessions } from "../ask/waiting";
import type { App } from "../shell/AppContext";
import {
  type ListModel,
  type ProjectGroup,
  type RowNode,
  buildList,
  folderName,
} from "./listModel";

function asObject(body: unknown): Record<string, unknown> {
  if (typeof body === "object" && body !== null && !Array.isArray(body)) {
    return body as Record<string, unknown>;
  }
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function text(value: unknown, name: string): string {
  if (typeof value === "string" && value !== "") return value;
  throw new UiControlError("invalidParams", `${name} must be a non-empty string`);
}

function flag(value: unknown, name: string): boolean {
  if (typeof value === "boolean") return value;
  throw new UiControlError("invalidParams", `${name} must be true or false`);
}

const rowJson = (n: RowNode): unknown => ({
  id: n.session.id,
  title: n.session.title,
  badge: n.session.badge,
  unread: n.session.unread,
  pinned: n.session.pinned,
  archived: n.session.archived,
  localOnly: n.session.localOnly,
  stuck: n.session.stuck,
  continuedIn: n.session.continuedIn,
  chip: n.chip,
  children: n.children.map(rowJson),
});

const groupJson = (p: ProjectGroup): unknown => ({
  key: p.key,
  id: p.id,
  name: p.name,
  path: p.path,
  folderName: p.path === null ? null : folderName(p.path),
  running: p.running,
  count: p.count,
  filed: p.filed,
  unread: p.unread,
  rows: p.rows.map(rowJson),
});

/** The list as the UI shows it, as plain JSON. */
function listJson(app: App): unknown {
  const state = app.core?.store.get();
  const waiting = state === undefined ? new Set<string>() : waitingSessions(state);
  const all = Object.values(state?.sessions ?? {}).map((entry) =>
    summarize(entry, state?.stuck ?? {}, waiting),
  );
  const model: ListModel = buildList(
    all,
    {
      query: app.shell.store.get().filter,
      showArchived: state?.index.includeArchived ?? false,
    },
    app.projects?.groups.store.get().data ?? null,
  );
  return {
    needs: model.needs.map((s) => s.id),
    pinned: model.pinned.map(rowJson),
    grouped: model.grouped,
    projects: model.projects.map(groupJson),
    unfiled: model.unfiled.map(rowJson),
    total: model.total,
    hiddenArchived: model.hiddenArchived,
    index: {
      pages: state?.index.pages ?? 0,
      nextCursor: state?.index.nextCursor ?? null,
      includeArchived: state?.index.includeArchived ?? false,
      continueSessionId: state?.index.continueSessionId ?? null,
    },
    filter: app.shell.store.get().filter,
    // temporary sessions (deleted when Vibe Work quits) and the new-chat view's switch.
    temporary: app.sessionTools?.store.get().temporary ?? [],
    newChatTemporary: app.sessionTools?.store.get().newChatTemporary ?? false,
  };
}

function nullableText(value: unknown, name: string): string | null {
  if (value === null) return null;
  return text(value, name);
}

/** The project actions of the `sessions` route; `null` for an action that is not one of them. */
async function projectAction(
  app: App,
  action: unknown,
  input: Record<string, unknown>,
): Promise<Record<string, unknown> | null> {
  const groups = app.projects?.groups;
  if (groups === undefined) {
    throw new UiControlError("serverUnavailable", "this window has no connection to the core");
  }
  const project = () => text(input["projectId"], "projectId");
  switch (action) {
    case "projectCreate":
      return { project: await groups.create(text(input["name"], "name")) };
    case "projectRename":
      await groups.rename(project(), text(input["name"], "name"));
      return {};
    case "projectDelete":
      return { moved: await groups.remove(project()) };
    case "projectMove": {
      const ids = input["sessionIds"];
      if (!Array.isArray(ids) || !ids.every((i) => typeof i === "string" && i !== "")) {
        throw new UiControlError("invalidParams", "sessionIds must be a list of session ids");
      }
      return {
        moved: await groups.move(ids as string[], nullableText(input["projectId"], "projectId")),
      };
    }
    case "projectPlace":
      await groups.place(project(), nullableText(input["before"] ?? null, "before"));
      return {};
    case "projectStart": {
      const id = project();
      const folder = await startFolderFor(app.core, groups, id, input["folder"]);
      return { sessionId: await groups.startIn(id, folder) };
    }
    case "projectFolder":
      // Never the native picker for the Debug API: no remote caller can answer it, and it holds the UI's request
      // queue until a person does.
      if (!("folder" in input))
        throw new UiControlError(
          "invalidParams",
          "folder is required: a native folder picker cannot be answered through the Debug API (give a path, or null to clear)",
        );
      await groups.setFolder(project(), nullableText(input["folder"], "folder"));
      return {};
    case "projectsRefresh":
      await groups.reconcile();
      return {};
    default:
      return null;
  }
}

/** Registers the routes; returns the function that removes them. */
export function registerSessionRoutes(app: App): () => void {
  const { core, sessions, shell, notifier } = app;
  const need = () => {
    if (core === null || sessions === null) {
      throw new UiControlError("serverUnavailable", "this window has no connection to the core");
    }
    return { core, sessions };
  };
  const removers = [
    app.uiControl.register("sessions", async (body) => {
      const input = asObject(body);
      const action = input["action"] ?? "read";
      if (action === "read") return listJson(app);
      const { core: c, sessions: act } = need();
      const id = () => text(input["sessionId"], "sessionId");
      switch (action) {
        case "filter":
          shell.actions.setFilter(typeof input["text"] === "string" ? input["text"] : "");
          break;
        case "archived":
          await c.setIncludeArchived(flag(input["show"], "show"));
          break;
        case "more":
          await c.loadMore();
          break;
        case "refresh":
          await c.refresh();
          break;
        case "open":
          await act.open(id());
          break;
        case "rename":
          await act.rename(id(), text(input["title"], "title"));
          break;
        case "pin":
          await act.pin(id(), flag(input["pinned"], "pinned"));
          break;
        case "archive":
          await act.archive(id(), flag(input["archived"], "archived"));
          break;
        case "delete": {
          // what the deleted session left running (no question is asked for a Debug API delete).
          const sessionId = id();
          const reason = act.deleteRefusal(sessionId);
          if (reason !== null) return { deleted: false, reason, list: listJson(app) };
          const deleted = await act.remove(sessionId);
          const leftRunning = (app.leftRunning?.store.get().items ?? []).filter(
            (item) => item.sessionId === sessionId,
          );
          return { deleted, leftRunning, list: listJson(app) };
        }
        case "retryReopen":
          return { reopened: await act.retryReopen(id()), list: listJson(app) };
        case "cancelReopen":
          return { cancelled: await act.cancelReopen(id()), list: listJson(app) };
        case "continueNew":
          return { sessionId: await act.continueInNew(id()), list: listJson(app) };
        case "temporaryChat":
          // the new-chat view's "Temporary chat" switch, as a click sets it.
          app.sessionTools?.setNewChatTemporary(flag(input["on"], "on"));
          break;
        case "toggleGroup":
          shell.actions.toggleProject(text(input["key"], "key"));
          break;
        default: {
          const answer = await projectAction(app, action, input);
          if (answer === null) throw new UiControlError("invalidParams", "unknown action");
          return { ...answer, list: listJson(app) };
        }
      }
      return listJson(app);
    }),
    app.uiControl.register("notifications", (body) => {
      const input = asObject(body);
      const action = input["action"] ?? "read";
      if (action === "level") {
        const level = NOTIFY_LEVELS.find((l) => l === input["level"]);
        if (level === undefined) {
          throw new UiControlError(
            "invalidParams",
            `level must be one of ${NOTIFY_LEVELS.join(", ")}`,
          );
        }
        notifier.setLevel(level);
      } else if (action === "focus") {
        const focused = input["focused"];
        if (focused !== null && typeof focused !== "boolean") {
          throw new UiControlError("invalidParams", "focused must be true, false or null");
        }
        notifier.setFocusOverride(focused);
      } else if (action !== "read") {
        throw new UiControlError("invalidParams", "action must be read, level or focus");
      }
      const s = notifier.store.get();
      return {
        level: s.level,
        focusOverride: s.focusOverride,
        error: s.error,
        log: s.log,
        bell: app.toasts.history.get(),
      };
    }),
  ];
  return () => {
    for (const remove of removers) remove();
  };
}
