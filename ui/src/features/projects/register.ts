/**
 * Plugs the Projects overview into the app: the palette entry "Open the Projects overview" and the
 * `ui/projects` route that lets an external agent read and drive the page the way a user does. The Rust
 * side lists the route (`crates/vw-host/src/host.rs` PAGES_ROUTES).
 * Route actions (body `{action,...}`): `state` (default: the page's projects with their cards and health, the cards
 * by folder as before, the filter, the selection), `open`, `close`, `refresh`, `select {projectId|null}`,
 * `filter {status}`, `query {text}`, `openHandbook {projectId, path?}`, `start {projectId}`, `migrate {projectId}`,
 * `newSession {projectId, folder?}` (for a project without a main folder `folder` names where to run, a path, or it is
 * `invalidParams`: the native folder picker cannot be answered remotely), `reveal {projectId}`, `chooseFolder {projectId, folder}` (`folder` is required: a path, or
 * `null` to clear), `newProject {name}`. A project is named by `projectId`; `project`
 * (a main folder, the key) is still accepted and means the first project with that main folder.
 * Depends on: state/projects, model.ts, state/uiControl, AppContext types. Called by: main.tsx, UI tests.
 * Invariants: bodies are validated (`invalidParams`); while a route request is served every core call carries the
 * Debug API origin (`uiControl` lowering); the route never writes a file.
 */
import { term } from "../../vocabulary";
import { startFolderFor } from "../../state/projectStartFolder";
import type { StatusFilter } from "../../state/projects";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import {
  STATUS_ORDER,
  displayName,
  healthOf,
  itemStatus,
  overviewItems,
  projectStatus,
} from "./model";

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function text(value: unknown, name: string): string {
  if (typeof value !== "string" || value === "")
    throw new UiControlError("invalidParams", `${name} must be a non-empty string`);
  return value;
}

const FILTERS: readonly StatusFilter[] = ["all", ...STATUS_ORDER];

/** Registers the palette entry and the route; returns the function that removes them. */
export function registerProjects(app: App): () => void {
  const projects = app.projects;
  if (projects === null) return () => undefined;
  const groups = () => projects.groups.store.get().data;
  const state = () => {
    const s = projects.store.get();
    const items = overviewItems(s.cards ?? [], groups());
    return {
      open: s.open,
      loading: s.loading,
      error: s.error,
      elapsedMs: s.elapsedMs,
      filter: s.filter,
      query: s.query,
      selected: s.selected,
      projects: items.map((i) => ({
        id: i.project.id,
        name: i.project.name,
        mainFolder: i.project.mainFolder,
        sessions: i.project.sessions,
        status: itemStatus(i),
        handbook: i.card === null ? null : displayName(i.card),
        health: i.card === null ? null : healthOf(i.card, s.hubs[i.card.project]).text,
      })),
      cards: (s.cards ?? []).map((c) => ({
        project: c.project,
        name: displayName(c),
        state: c.state,
        status: projectStatus(c),
        plans: c.plans.map((p) => ({
          path: p.path,
          title: p.title,
          steps: p.steps.map((x) => x.status),
        })),
        health: healthOf(c, s.hubs[c.project]).text,
      })),
    };
  };
  /** The project a body names: `projectId`, else `project` = a main folder (the key). */
  const target = (input: Record<string, unknown>): string => {
    if (typeof input["projectId"] === "string" && input["projectId"] !== "")
      return input["projectId"];
    const folder = text(input["project"], "projectId (or project)");
    const found = groups()?.projects.find((p) => p.mainFolder === folder);
    if (found === undefined)
      throw new UiControlError("notFound", "no project has this main folder");
    return found.id;
  };
  const removers = [
    app.commands.register({
      id: "projects.open",
      label: "Open the Projects overview",
      section: term("workNames", "projectTab").name,
      keywords: "projects overview handbook plans workspace",
      run: () => {
        app.connectors?.actions.close();
        void projects.open();
      },
    }),
    app.uiControl.register("projects", async (body) => {
      const input = asObject(body);
      const project = () => target(input);
      switch (input["action"] ?? "state") {
        case "state":
          return state();
        case "open":
          app.connectors?.actions.close();
          await projects.open();
          return state();
        case "close":
          projects.close();
          return state();
        case "refresh":
          await projects.refresh();
          return state();
        case "select":
          projects.select(
            input["projectId"] === null || input["project"] === null ? null : project(),
          );
          return state();
        case "filter": {
          const status = FILTERS.find((f) => f === input["status"]);
          if (status === undefined)
            throw new UiControlError(
              "invalidParams",
              `status must be one of ${FILTERS.join(", ")}`,
            );
          projects.setFilter(status);
          return state();
        }
        case "query":
          projects.setQuery(typeof input["text"] === "string" ? input["text"] : "");
          return state();
        case "openHandbook":
          await projects.openHandbook(
            project(),
            typeof input["path"] === "string" ? input["path"] : null,
          );
          return { session: app.shell.store.get().selected, ...state() };
        case "start":
          await projects.startHandbook(project());
          return { session: app.shell.store.get().selected, ...state() };
        case "migrate":
          await projects.migrate(project());
          return { session: app.shell.store.get().selected, ...state() };
        case "newSession": {
          const id = project();
          // A project without a main folder would open the native folder picker, which no remote caller can answer:
          // the request names the folder instead (`folder`), or is refused by name.
          const folder = await startFolderFor(app.core, projects.groups, id, input["folder"]);
          await projects.newSession(id, folder);
          return { session: app.shell.store.get().selected, ...state() };
        }
        case "reveal":
          await projects.reveal(project());
          return state();
        case "chooseFolder": {
          const folder = input["folder"];
          // Never the native picker for the Debug API: no remote caller can answer it, and it holds the UI's
          // request queue until a person does.
          if (folder === undefined)
            throw new UiControlError(
              "invalidParams",
              "folder is required: a native folder picker cannot be answered through the Debug API (give a path, or null to clear)",
            );
          if (folder !== null && typeof folder !== "string")
            throw new UiControlError("invalidParams", "folder must be a path or null");
          await projects.chooseFolder(project(), folder);
          return state();
        }
        case "newProject":
          return { project: await projects.groups.create(text(input["name"], "name")), ...state() };
        default:
          throw new UiControlError(
            "invalidParams",
            "action must be state, open, close, refresh, select, filter, query, openHandbook, start, migrate, newSession, reveal, chooseFolder or newProject",
          );
      }
    }),
  ];
  return () => {
    for (const remove of removers) remove();
    projects.stop();
  };
}
