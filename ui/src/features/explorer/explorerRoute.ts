/**
 * The `ui/explorer` UI-control route (listed by the Rust host in `crates/vw-host/src/host.rs` CHANGES_ROUTES): the
 * Files tab (the folder explorer), every action its controls have, through the same models a click uses. Acting opens the Files tab first, as a person would see it.
 * - `state` (default): the folder and Places, the view, the summary line, the tree rows as drawn (expanded
 *   folders, changed marks, "more" rows), the search and its results, History (commits, Show 50 more, the
 *   changes since the last commit), an open commit's files, and the folder actions before a session;
 * - `place {folder}`, `view {view: files|history}`, `expand {path}`, `collapse {path}`, `more {path?}` (a folder's
 *   next page), `changedOnly {on}`, `showIgnored {on}`, `refresh`;
 * - `search {query}` (waits for the results), `openFirst` (Enter in the search field), `open {path, line?}` (a tree
 *   row, a result, a changed or commit file: opens the File tab, never starts a session);
 * - `historyMore`, `version {id}` (a commit row or a commit result), `back`, `pending {open}`;
 * - before a session (History > Changes since the last commit): `review`, `closeReview`, `commit` (the Commit
 *   dialog; finish it with `ui/git` and the answered `commitKey`), `askRevert` / `cancelRevert` / `confirmRevert`,
 *   `enableHistory` / `identity {name, email}` / `cancelHistory` / `confirmHistory`; in a session `reviewSession`
 *   opens the Changes tab instead.
 * Depends on: the explorer store, panelModel, workspaceModel, FileTree.visibleRows, routeInput.
 * Called by: git-browser/register.
 * Invariants: bodies are validated (`invalidParams` names the field); a place must be one of the Places choices
 * (an outside-project copy without a session is refused like its disabled row); Revert and Turn on version
 * history keep their confirmation step and the core's checks (head, fingerprint, new-file list; `confirmed` and a
 * valid identity) — the Debug API acts as the user, so no native dialog is added or skipped.
 */
import type { GitBrowserSnapshot } from "../../protocol/core";
import { panelOf } from "../../state/shellState";
import {
  bodyObject,
  boolField,
  isStateRead,
  oneOf,
  settleWithin,
  textField,
} from "../../state/routeInput";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { visibleRows } from "./FileTree";
import { openInPanel, panelContext, type PanelContext, type PlaceStore } from "./panelModel";
import { shownResults, type Explorer, type ExplorerState } from "./store";
import { workspaceModelOf } from "./workspaceModel";

/** Most tree rows in one answer (the window virtualizes; `rowsTotal` says how many there are). */
const ROW_LIMIT = 500;
/** How long an action waits for the reads it started (folders, status, history, search, a commit). */
const SETTLE_MS = 8_000;

/** Folder actions whose failure is the answer (the window shows it under the buttons). */
const WRITES = ["review", "commit", "askRevert", "confirmRevert", "confirmHistory"];

/** What the Files tab shows the route: the snapshot and the panel key BrowserPanel renders with. */
export type ExplorerSource = () => { snapshot: GitBrowserSnapshot | null; key: string };

const versionWords = (v: ExplorerState["versions"][number]) => ({
  id: v.id,
  subject: v.subject,
  who: v.who,
  time: v.time,
  published: v.published ?? null,
});

/** The route's answer. */
function state(app: App, explorer: Explorer, context: PanelContext): unknown {
  const s = explorer.store.get();
  const ws = workspaceModelOf(app, explorer).store.get();
  const changed = new Set((s.status?.files ?? []).map((f) => f.path));
  const rows = visibleRows(s);
  const shell = app.shell.store.get();
  return {
    shown: panelOf(shell).open && panelOf(shell).active === "git",
    folder: s.folder,
    panelFolder: context.folder,
    places: context.places,
    session: context.session,
    view: s.view,
    summary:
      s.status === null
        ? null
        : {
            branch: s.status.branch,
            changed: s.status.changedFiles + s.status.untrackedFiles,
            ahead: s.status.ahead,
            upstream: s.status.upstream,
            commits: s.commitCount,
            latest: s.latest === null ? null : versionWords(s.latest),
          },
    historyNote: s.historyNote,
    files: s.paths.length,
    indexing: s.indexing,
    indexLimited: s.indexLimited,
    changedOnly: s.changedOnly,
    showIgnored: s.showIgnored,
    ignored: s.showIgnored ? [] : (s.folders[""]?.ignored ?? []),
    loading: s.loadingFolders.length > 0 || s.statusLoading,
    rows: rows.slice(0, ROW_LIMIT).map((r) => ({
      path: r.path,
      directory: r.directory,
      depth: r.depth,
      ...(r.more === true ? { more: true } : {}),
      expanded: r.directory && s.expanded.includes(r.path),
      changed: r.directory
        ? [...changed].some((p) => p.startsWith(`${r.path}/`))
        : changed.has(r.path),
    })),
    rowsTotal: rows.length,
    search:
      s.query === ""
        ? null
        : {
            query: s.query,
            searching: s.searching,
            // What the window shows (Search in this folder).
            ...shownResults(s),
            commits: s.searchVersions.map(versionWords),
            limited: s.searchLimited,
          },
    history: {
      loaded: s.historyLoaded,
      loading: s.historyLoading,
      commits: s.versions.map(versionWords),
      more: s.cursor !== null,
      pendingOpen: s.pendingOpen,
      pending: [...changed],
    },
    version:
      s.version === null
        ? null
        : {
            ...versionWords(s.version),
            files: s.versionFiles,
            loading: s.versionLoading,
            limited: s.versionLimited,
          },
    workspace: {
      commitKey: context.folder === null ? null : `files-workspace:${context.folder}`,
      review:
        ws.review === null
          ? null
          : {
              files: ws.review.view.files.map((f) => ({ path: f.path, status: f.status })),
              fingerprint: ws.review.view.fingerprint,
            },
      confirm: ws.confirm,
      busy: ws.busy,
      error: ws.error,
      init: ws.init,
      plan: ws.plan,
      identity: ws.identity,
    },
    error: s.error,
  };
}

/** True while the explorer still reads what the last action asked for. */
function reading(s: ExplorerState): boolean {
  return (
    s.loadingFolders.length > 0 ||
    s.statusLoading ||
    s.historyLoading ||
    s.searching ||
    s.versionLoading
  );
}

/** One `ui/explorer` request. */
async function route(
  app: App,
  explorer: Explorer,
  places: PlaceStore,
  source: ExplorerSource,
  body: unknown,
): Promise<unknown> {
  const input = bodyObject(body);
  const action = input["action"] ?? "state";
  const read = () => {
    const { snapshot, key } = source();
    return { snapshot, key, context: panelContext(app, snapshot, places.get(), key) };
  };
  if (action === "state") return state(app, explorer, read().context);
  // Acting shows the Files tab on the panel's folder first, like a person who opens it.
  app.shell.actions.openTab("git");
  const { snapshot, key } = read();
  let { context } = read();
  const show = async () => {
    if (explorer.store.get().folder !== context.folder) explorer.choose(context.folder);
    await settleWithin(() => !reading(explorer.store.get()), SETTLE_MS);
  };
  await show();
  const ws = workspaceModelOf(app, explorer);
  const s = () => explorer.store.get();
  const beforeSession = (what: string) => {
    if (context.session)
      throw new UiControlError(
        "invalidParams",
        `${what} is offered before a session; in a session use the Changes tab (ui/changes, ui/git)`,
      );
  };
  const inRepository = () => {
    if (s().status === null)
      throw new UiControlError(
        "invalidParams",
        "this folder has no version history (enableHistory)",
      );
    explorer.setView("history");
    explorer.setPending(true);
  };
  switch (action) {
    case "place": {
      const folder = textField(input, "folder");
      const option = context.places.find((p) => p.value === folder);
      if (option === undefined)
        throw new UiControlError("notFound", "folder is not one of the Places");
      if (option.disabled === true)
        throw new UiControlError("denied", "this copy is outside the project and has no session");
      places.set({ key, place: folder });
      ws.closeInit();
      ({ context } = read());
      await show();
      break;
    }
    case "view":
      explorer.setView(oneOf(input, "view", ["files", "history"] as const));
      break;
    case "expand":
    case "collapse": {
      const path = textField(input, "path");
      const row = visibleRows(s()).find((r) => r.path === path && r.directory);
      if (row === undefined)
        throw new UiControlError("notFound", "no folder row with that path is shown");
      if (s().expanded.includes(path) !== (action === "expand")) explorer.toggle(path);
      break;
    }
    case "more": {
      const path = typeof input["path"] === "string" ? input["path"] : "";
      if (s().folders[path]?.next == null)
        throw new UiControlError("invalidParams", "that folder has no more entries to show");
      await explorer.loadFolder(path, true);
      break;
    }
    case "changedOnly":
      explorer.changedOnly(boolField(input, "on"));
      break;
    case "showIgnored":
      explorer.showIgnored(boolField(input, "on"));
      break;
    case "refresh":
      explorer.refresh();
      break;
    case "search": {
      const query = input["query"];
      if (typeof query !== "string")
        throw new UiControlError("invalidParams", "query must be a string");
      explorer.query(query);
      break;
    }
    case "openFirst": {
      const first = shownResults(s());
      const version = s().searchVersions[0];
      if (first.names[0] !== undefined) {
        if (
          !(await openInPanel(
            app,
            explorer,
            context,
            snapshot,
            first.names[0],
            undefined,
            "debug-api",
          ))
        )
          throw new UiControlError("notFound", "the file could not be opened");
      } else if (first.hits[0] !== undefined) {
        const hit = first.hits[0];
        if (!(await openInPanel(app, explorer, context, snapshot, hit.path, hit.line, "debug-api")))
          throw new UiControlError("notFound", "the file could not be opened");
      } else if (version !== undefined) await explorer.openFound(version);
      else throw new UiControlError("invalidParams", "the search has no results");
      break;
    }
    case "open": {
      const path = textField(input, "path");
      const line = typeof input["line"] === "number" ? input["line"] : undefined;
      if (!(await openInPanel(app, explorer, context, snapshot, path, line, "debug-api")))
        throw new UiControlError(
          "notFound",
          "the file could not be opened (Refresh, then try again)",
        );
      break;
    }
    case "historyMore":
      if (s().cursor === null)
        throw new UiControlError("invalidParams", "History has no more commits");
      await explorer.history(true);
      break;
    case "version": {
      const id = textField(input, "id");
      const fromHistory = s().versions.find((v) => v.id === id);
      const found = s().searchVersions.find((v) => v.id === id);
      if (fromHistory !== undefined) {
        explorer.setView("history");
        await explorer.openVersion(fromHistory);
      } else if (found !== undefined) await explorer.openFound(found);
      else throw new UiControlError("notFound", "no shown commit has that id");
      break;
    }
    case "back":
      explorer.back();
      break;
    case "pending":
      explorer.setView("history");
      explorer.setPending(boolField(input, "open"));
      break;
    case "reviewSession":
      if (!context.session)
        throw new UiControlError("invalidParams", "no session is selected (use review)");
      app.panelTabs.setHidden("changes", false);
      app.shell.actions.openTab("changes");
      break;
    case "review":
      beforeSession("Review all changes");
      inRepository();
      await ws.review();
      break;
    case "closeReview":
      ws.closeReview();
      break;
    case "commit":
      beforeSession("Commit");
      inRepository();
      await ws.commit();
      break;
    case "askRevert":
      beforeSession("Revert all changes");
      inRepository();
      await ws.askRevert();
      break;
    case "cancelRevert":
      ws.cancelRevert();
      break;
    case "confirmRevert":
      if (!ws.store.get().confirm)
        throw new UiControlError(
          "invalidParams",
          "the Revert confirmation is not open (askRevert)",
        );
      await ws.confirmRevert();
      break;
    case "enableHistory":
      beforeSession("Turn on version history");
      if (s().status !== null)
        throw new UiControlError("invalidParams", "this folder already has version history");
      explorer.setView("history");
      ws.openInit();
      await settleWithin(
        () => ws.store.get().plan !== null || ws.store.get().error !== null,
        SETTLE_MS,
      );
      break;
    case "identity": {
      const name = textField(input, "name");
      const email = textField(input, "email");
      ws.setIdentity({ name, email });
      break;
    }
    case "cancelHistory":
      ws.closeInit();
      break;
    case "confirmHistory":
      if (ws.store.get().plan === null)
        throw new UiControlError("invalidParams", "the Turn on version history dialog is not open");
      await ws.confirmInit();
      break;
    default:
      throw new UiControlError("invalidParams", `unknown action ${JSON.stringify(action)}`);
  }
  // A folder action that could not finish says so, as the window does under its buttons.
  const failed = ws.store.get().error;
  if (failed !== null && typeof action === "string" && WRITES.includes(action))
    throw new UiControlError("denied", failed);
  await settleWithin(() => !reading(explorer.store.get()), SETTLE_MS);
  return state(app, explorer, read().context);
}

/** Registers `ui/explorer`; returns the function that removes it. */
export function registerExplorerRoute(
  app: App,
  explorer: Explorer,
  places: PlaceStore,
  source: ExplorerSource,
): () => void {
  return app.uiControl.register(
    "explorer",
    (body) => route(app, explorer, places, source, body),
    isStateRead,
  );
}
