/**
 * Registers the Files tab (the saved-project Git tree under History, live snapshot refresh), the ui/git-browser
 * route and the ui/explorer route (`../explorer/explorerRoute.ts`). Called by main.
 */
import { term } from "../../vocabulary";
import { Selector, type SelectorOption } from "../../components/Selector";
import { useEffect, type ReactElement } from "react";
import type { GitBrowserSnapshot } from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import { createStore, useStore } from "../../state/createStore";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { adapterAction, adapterFile } from "./routing";
import { pointGitFile } from "./point";
import { reposOf } from "./adapter";
import { GitBrowser } from "./GitBrowser";
import { FilesPanel } from "../explorer/FilesPanel";
import { createExplorer, type Explorer } from "../explorer/store";
import { createPlaceStore, panelContext, type PlaceStore } from "../explorer/panelModel";
import { registerExplorerRoute } from "../explorer/explorerRoute";
import { bindFiles } from "../explorer/filesBridge";
import { setFilesTabSource } from "../files/panelRead";
import { actionStep, nextStep, stateWords, type BrowserAction, type BrowserTarget } from "./model";

type View = {
  projectId: string | null;
  snapshot: GitBrowserSnapshot | null;
  loading: boolean;
  error: string | null;
};

const ACTIONS: readonly BrowserAction[] = ["commit", "push", "pr", "bring", "new-chat", "new-task"];

function routeNode(target: BrowserTarget) {
  return {
    id: target.id,
    name: target.name,
    path: target.path,
    branch: target.branch ?? null,
    words: stateWords(target),
    files: target.files,
    primary: nextStep(target),
    actions:
      target.kind === "copy" && target.restricted
        ? [nextStep(target)]
        : ACTIONS.map((action) => actionStep(target, action)),
  };
}

function currentProject(app: App): string | null {
  const groups = app.projects?.groups.store.get().data;
  if (groups === null || groups === undefined) return null;
  const shell = app.shell.store.get();
  if (shell.selected !== null) return groups.members[shell.selected] ?? null;
  const selected = app.projects?.store.get().selected;
  if (
    selected !== null &&
    selected !== undefined &&
    groups.projects.some((p) => p.id === selected && p.mainFolder !== null)
  )
    return selected;
  const folder = shell.newFolder;
  return groups.projects.find((p) => p.mainFolder !== null && p.mainFolder === folder)?.id ?? null;
}

function BrowserPanel({
  app,
  view,
  watch,
  explorer,
  places,
}: {
  app: App;
  explorer: Explorer;
  places: PlaceStore;
  view: ReturnType<typeof createStore<View>>;
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
          <summary data-testid="explorer.repositories">Repositories and session copies</summary>
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

/** Registers one controller for the panel and route; a project switch releases its prior watch. */
export function registerGitBrowser(app: App): () => void {
  if (app.core === null || app.projects === null) return () => undefined;
  const core = app.core;
  const explorer = createExplorer(core);
  const places = createPlaceStore();
  const view = createStore<View>({ projectId: null, snapshot: null, loading: false, error: null });
  let watched: string | null = null;
  let version = 0;
  let eventTimer: ReturnType<typeof setTimeout> | null = null;
  let lastRead = 0;
  let inFlight: {
    projectId: string;
    op: "read" | "watch";
    call: number;
    promise: Promise<void>;
  } | null = null;
  const run = async (projectId: string, op: "read" | "watch" | "unwatch", background = false) => {
    const answer = await core.run(
      { type: "gitBrowser", request: { op, projectId } },
      background ? { origin: "window" } : undefined,
    );
    if (answer.type !== "gitBrowser") throw new Error("Git browser answer unavailable.");
    return answer.result;
  };
  const read = (projectId: string, op: "read" | "watch"): Promise<void> => {
    if (inFlight?.projectId === projectId && inFlight.op === op) return inFlight.promise;
    const call = ++version;
    view.set((s) => ({
      ...s,
      projectId,
      snapshot: s.projectId === projectId ? s.snapshot : null,
      loading: true,
      error: null,
    }));
    const promise = (async () => {
      try {
        const result = await run(projectId, op, true);
        if (call === version && result.kind === "snapshot")
          view.set({ projectId, snapshot: result.snapshot, loading: false, error: null });
      } catch (error) {
        if (call === version)
          view.set((s) => ({
            ...s,
            projectId,
            loading: false,
            error: toCallError(error).error.message,
          }));
      } finally {
        lastRead = Date.now();
        if (inFlight?.call === call) inFlight = null;
      }
    })();
    inFlight = { projectId, op, call, promise };
    return promise;
  };
  const watch = async (projectId: string | null) => {
    if (projectId === null) {
      const old = watched;
      watched = null;
      version++;
      if (eventTimer !== null) clearTimeout(eventTimer);
      eventTimer = null;
      view.set({ projectId: null, snapshot: null, loading: false, error: null });
      if (old !== null) void run(old, "unwatch", true).catch(() => undefined);
      return;
    }
    if (watched === projectId) {
      if (view.get().snapshot === null || view.get().error !== null) await read(projectId, "read");
      return;
    }
    const old = watched;
    watched = projectId;
    if (eventTimer !== null) clearTimeout(eventTimer);
    eventTimer = null;
    if (old !== null) void run(old, "unwatch", true).catch(() => undefined);
    await read(projectId, "watch");
  };
  const stopEvents = core.listen((event) => {
    if (event.type !== "gitBrowserChanged" || event.projectId !== watched || eventTimer !== null)
      return;
    const projectId = event.projectId;
    const delay = Math.max(400, lastRead + 1_000 - Date.now());
    eventTimer = setTimeout(() => {
      eventTimer = null;
      if (watched === projectId) void read(projectId, "read");
    }, delay);
  });
  const removeBody = app.panelTabs.register("git", () => (
    <BrowserPanel app={app} view={view} watch={watch} explorer={explorer} places={places} />
  ));
  // The same snapshot and panel key BrowserPanel renders with, for the ui/explorer route.
  const removeExplorerRoute = registerExplorerRoute(app, explorer, places, () => {
    const projectId = currentProject(app);
    const state = view.get();
    return {
      snapshot: state.projectId === projectId ? state.snapshot : null,
      key: app.shell.store.get().selected ?? projectId ?? "new",
    };
  });
  const search = () => {
    app.shell.actions.openTab("git");
    setTimeout(
      () => document.querySelector<HTMLInputElement>('[data-testid="explorer.search"]')?.focus(),
      50,
    );
  };
  const removeSearch = app.commands.register({
    id: "files.search",
    label: "Search files",
    section: "Files",
    keys: "Mod+Shift+F",
    keywords: "folder text commits",
    run: search,
  });
  const removeHistory = app.commands.register({
    id: "files.history",
    label: "Show history",
    section: "Files",
    keywords: "git commits versions",
    run: () => {
      app.shell.actions.openTab("git");
      explorer.setView("history");
    },
  });
  const removeKeys = app.shortcuts.register({
    id: "files.search",
    keys: "Mod+Shift+F",
    description: "Search files, text and commits",
    run: search,
  });
  const removeRoute = app.uiControl.register("git-browser", async (body) => {
    const params = body === undefined || body === null ? {} : body;
    if (typeof params !== "object" || Array.isArray(params))
      throw new UiControlError("invalidParams", "body must be an object");
    const input = params as Record<string, unknown>;
    const requested = input["projectId"];
    if (requested !== undefined && (typeof requested !== "string" || requested === ""))
      throw new UiControlError("invalidParams", "projectId must be a saved project id");
    const groups = app.projects?.groups.store.get().data;
    if (requested !== undefined && !groups?.projects.some((p) => p.id === requested))
      throw new UiControlError("notFound", "projectId is not a saved project");
    const projectId = typeof requested === "string" ? requested : currentProject(app);
    if (projectId === null)
      throw new UiControlError("invalidParams", "choose a saved project first");
    const action = input["action"] ?? "read";
    if (action !== "read" && action !== "click")
      throw new UiControlError("invalidParams", "action must be read or click");
    let result: Awaited<ReturnType<typeof run>>;
    try {
      result = await run(projectId, "read");
    } catch (error) {
      const failure = toCallError(error).error;
      throw new UiControlError(failure.code, failure.message);
    }
    if (result.kind !== "snapshot")
      throw new UiControlError("serverUnavailable", "Git browser snapshot unavailable");
    const snapshot = result.snapshot;
    if (action === "click") {
      const nodeId = input["nodeId"];
      const operation = input["target"];
      if (typeof nodeId !== "string" || typeof operation !== "string")
        throw new UiControlError("invalidParams", "nodeId and target are required");
      const node = snapshot.nodes.find((n) => n.id === nodeId);
      if (node === undefined) throw new UiControlError("notFound", "node is not in this project");
      const target = reposOf(snapshot)
        .flatMap((r) => [r, ...r.copies])
        .find((r) => r.id === nodeId);
      if (target === undefined) throw new UiControlError("denied", "this node has no actions");
      if (operation === "file") {
        const path = input["path"];
        if (typeof path !== "string" || !target.files.some((f) => f.path === path))
          throw new UiControlError("notFound", "file is not in this node");
        await adapterFile(app, target, path, "debug-api");
      } else {
        const chosen =
          operation === "primary" ? nextStep(target).action : (operation as BrowserAction);
        if (
          !["commit", "push", "pr", "bring", "new-chat", "new-task", "open-chat"].includes(chosen)
        )
          throw new UiControlError("invalidParams", "unknown target");
        const reason = actionStep(target, chosen).reason;
        if (reason !== null) throw new UiControlError("denied", reason);
        await adapterAction(app, target, chosen, "debug-api");
      }
    }
    return {
      snapshot,
      repos: reposOf(snapshot).map((r) => ({ ...routeNode(r), copies: r.copies.map(routeNode) })),
    };
  });
  // The file menus elsewhere (chat links, file tabs, Changes) reach this Files tab through the bridge.
  const unbind = bindFiles(app, {
    explorer,
    showFolder: (folder) => {
      if (explorer.store.get().folder === folder) return;
      places.set({
        key: app.shell.store.get().selected ?? currentProject(app) ?? "new",
        place: folder,
      });
      explorer.choose(folder);
    },
  });
  // What the agent's panel_read reports about the Files tab: the folder, the view and the search text. The one
  // explorer follows the selected session only while the Files tab is on screen, so it may still hold another
  // session's folder: then nothing is reported. The source is asked only for the selected session, the one
  // panelContext computes.
  setFilesTabSource((sessionId) => {
    const s = explorer.store.get();
    const projectId = currentProject(app);
    const state = view.get();
    const snapshot = state.projectId === projectId ? state.snapshot : null;
    if (s.folder !== panelContext(app, snapshot, places.get(), sessionId).folder) return null;
    return { folder: s.folder, view: s.view, search: s.query };
  });
  return () => {
    unbind();
    setFilesTabSource(null);
    version++;
    if (eventTimer !== null) clearTimeout(eventTimer);
    stopEvents();
    removeRoute();
    removeExplorerRoute();
    removeBody();
    explorer.stop();
    removeSearch();
    removeHistory();
    removeKeys();
    if (watched !== null) void run(watched, "unwatch", true).catch(() => undefined);
  };
}
