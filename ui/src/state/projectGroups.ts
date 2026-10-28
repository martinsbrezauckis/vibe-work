/**
 * The user's project groups as the window sees them: the projects in the user's order and the session ->
 * project map, read from the core's `projects` command and kept current by the `projectsChanged` event, plus the
 * list's transient UI state (which project is being renamed or created, the "Move to project…" menu, the delete
 * confirmation). Every change is a core command; nothing is shown as done before the core answers.
 * Depends on: coreStore (`run`, `listen`), shellStore (selection, collapse migration), toastStore, createStore,
 * protocol types. Called by: state/projects (holds it as `groups`), the session list, the palette, the `sessions`
 * UI route, the Projects overview.
 * Invariants:
 * - `data` is `null` until the first answer (the list then groups by folder, as before projects existed).
 * - Refetches are coalesced: one `list` in flight, one more queued at most.
 * - Deleting a project never deletes a session (the core re-files them); the window asks first in its own dialog.
 * - Failures become one toast with the core's message (a declined Debug API dialog shows none); actions never throw.
 */
import { toCallError } from "../protocol/client/types";
import type { ProjectView, ProjectsRequest, ProjectsResult } from "../protocol/core";
import type { CoreController } from "./coreStore";
import { requestComposerFocus } from "./composerFocus";
import { createStore } from "./createStore";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";

/** What the core said about the groups. */
export interface GroupsData {
  /** Projects in the user's order. */
  projects: readonly ProjectView[];
  /** Session id -> project id; `null` = not in a project; absent = not filed yet. */
  members: Readonly<Record<string, string | null>>;
}

/** Where the "Move to project…" menu opens and for which sessions. */
interface MoverState {
  sessionIds: readonly string[];
  at: { x: number; y: number };
}

/** The store's state. */
export interface GroupsState {
  data: GroupsData | null;
  error: string | null;
  /** The project whose name field is open. */
  renaming: string | null;
  /** The "New project" name field is open. */
  creating: boolean;
  mover: MoverState | null;
  /** The project whose delete confirmation is open. */
  deleting: string | null;
}

const initial: GroupsState = {
  data: null,
  error: null,
  renaming: null,
  creating: false,
  mover: null,
  deleting: null,
};

/**
 * What a Debug API route answers (`invalidParams`) for a new chat in a project without a main folder when it names no
 * folder: the chat would open the native folder picker, which no remote caller can answer and which would keep the
 * request waiting for a person. Same rule as `projectFolder` and `chooseFolder`, which require the folder.
 */
export const NEEDS_MAIN_FOLDER =
  "this project has no main folder, so a new chat needs the folder to run in: give folder (a path), or set the project's folder first (projectFolder or chooseFolder with a path)";

/** Creates the controller. */
export function createProjectGroups(deps: { core: CoreController; shell: Shell; toasts: Toasts }) {
  const { core, shell, toasts } = deps;
  const store = createStore<GroupsState>(initial);
  const patch = (p: Partial<GroupsState>) => {
    store.set((s) => ({ ...s, ...p }));
  };
  const fail = (thrown: unknown): void => {
    const error = toCallError(thrown).error;
    if (error.code === "confirmationDeclined") return;
    toasts.push({ kind: "error", bellKind: "failure", text: error.message });
  };
  const run = async (request: ProjectsRequest): Promise<ProjectsResult> => {
    const result = await core.run({ type: "projects", request });
    if (result.type !== "projects") throw new Error(`unexpected ${result.type} result`);
    return result.result;
  };

  /** Copies the collapse mark of a folder group (before projects) to the project with that main folder, once. */
  const migrateCollapsed = (projects: readonly ProjectView[]) => {
    const collapsed = shell.store.get().collapsed;
    for (const p of projects) {
      const folder = p.mainFolder;
      if (folder !== null && collapsed[folder] === true) {
        if (collapsed[p.id] !== true) shell.actions.toggleProject(p.id);
        shell.actions.toggleProject(folder);
      }
    }
  };

  const apply = (result: ProjectsResult) => {
    if (result.type !== "list") return;
    migrateCollapsed(result.projects);
    patch({ data: { projects: result.projects, members: result.members }, error: null });
  };

  let inFlight: Promise<void> | null = null;
  let again = false;
  const fetchList = async (op: "list" | "refresh"): Promise<void> => {
    try {
      apply(await run({ op }));
    } catch (thrown) {
      patch({ error: toCallError(thrown).error.message });
    }
  };
  /** Reads the groups again (coalesced). */
  const refresh = (): Promise<void> => {
    if (inFlight !== null) {
      again = true;
      return inFlight;
    }
    // A refresh asked while one runs is folded into one more read after it (callers get the same promise).
    const loop = async (): Promise<void> => {
      await fetchList("list");
      if (again) {
        again = false;
        await loop();
      }
    };
    inFlight = loop().finally(() => {
      inFlight = null;
    });
    return inFlight;
  };

  /** Runs a change; refreshes after it (the event would too, this keeps tests and slow events honest). */
  const change = async (request: ProjectsRequest): Promise<ProjectsResult | null> => {
    try {
      const result = await run(request);
      await refresh();
      return result;
    } catch (thrown) {
      fail(thrown);
      return null;
    }
  };

  const projectOf = (id: string) => store.get().data?.projects.find((p) => p.id === id) ?? null;

  const controller = {
    store,
    refresh,
    /**
     * True when a new chat in project `id` would open the native folder picker (the project has no main folder).
     * Debug API routes refuse that first (`NEEDS_MAIN_FOLDER`): no remote caller can answer the picker.
     */
    needsFolderPicker(id: string): boolean {
      const project = projectOf(id);
      return project !== null && project.mainFolder === null;
    },
    /** Reconciles with Vibe's whole session list now (`refresh` op), then shows the result. */
    async reconcile(): Promise<void> {
      await fetchList("refresh");
    },
    /** Reads the groups and follows `projectsChanged` (and a server restart); returns the stop function. */
    start(): () => void {
      void refresh();
      return core.listen((event) => {
        if (event.type === "projectsChanged" || event.type === "lagged") void refresh();
        if (event.type === "server" && event.status.state === "running") void refresh();
      });
    },
    /** A name-only project (optionally with a main folder) at the top of the list. */
    async create(name: string, mainFolder?: string): Promise<ProjectView | null> {
      const clean = name.trim();
      if (clean === "") return null;
      const result = await change({
        op: "create",
        name: clean,
        ...(mainFolder === undefined ? {} : { mainFolder }),
      });
      return result?.type === "created" ? result.project : null;
    },
    async rename(id: string, name: string): Promise<void> {
      const clean = name.trim();
      if (clean === "" || projectOf(id)?.name === clean) return;
      await change({ op: "rename", projectId: id, name: clean });
    },
    /** Deletes a project; its sessions move to "Not in a project". Returns how many moved, `null` on failure. */
    async remove(id: string): Promise<number | null> {
      const result = await change({ op: "delete", projectId: id });
      return result?.type === "moved" ? result.moved : null;
    },
    /** Files sessions into `projectId` (`null`: out of every project). */
    async move(sessionIds: readonly string[], projectId: string | null): Promise<boolean> {
      if (sessionIds.length === 0) return false;
      const result = await change({ op: "move", sessionIds: [...sessionIds], projectId });
      return result !== null;
    },
    /** Moves a project before another (`null`: to the end). */
    async place(id: string, before: string | null): Promise<void> {
      if (id === before) return;
      await change({ op: "place", projectId: id, before });
    },
    /** One step up or down in the user's order (keyboard reorder). */
    async shift(id: string, step: -1 | 1): Promise<void> {
      const list = store.get().data?.projects ?? [];
      const at = list.findIndex((p) => p.id === id);
      if (at < 0) return;
      if (step === -1 && at > 0) await controller.place(id, list[at - 1]?.id ?? null);
      if (step === 1 && at < list.length - 1) await controller.place(id, list[at + 2]?.id ?? null);
    },
    async setFolder(id: string, folder: string | null): Promise<boolean> {
      return (await change({ op: "setFolder", projectId: id, folder })) !== null;
    },
    /** "Choose main folder…": the native picker, then the folder. */
    async chooseFolder(id: string): Promise<void> {
      let folder: string | null;
      try {
        folder = await core.client.pickFolder();
      } catch (thrown) {
        fail(thrown);
        return;
      }
      if (folder !== null) await controller.setFolder(id, folder);
    },
    /**
     * A new chat in the project's main folder (or a folder the user picks when it has none), filed by the core.
     * With `folder` (a Debug API request, which cannot answer the picker, names where) the chat runs there.
     * Expands the project, selects the new chat and focuses its composer; returns its id, null on failure/cancel.
     */
    async startIn(id: string, folder?: string): Promise<string | null> {
      const project = projectOf(id);
      if (project === null) return null;
      let cwd = folder ?? project.mainFolder;
      try {
        cwd ??= await core.client.pickFolder();
        if (cwd === null) return null;
        const result = await core.run({
          type: "projects",
          request: { op: "start", projectId: id, agentConfig: { cwd } },
        });
        if (result.type !== "session") return null;
        if (shell.store.get().collapsed[id] === true) shell.actions.toggleProject(id);
        shell.actions.selectSession(result.snapshot.sessionId);
        await refresh();
        requestComposerFocus(result.snapshot.sessionId);
        return result.snapshot.sessionId;
      } catch (thrown) {
        fail(thrown);
        return null;
      }
    },
    /** "New project…" from the move menu: a project named "New project" holding the sessions, then its name field. */
    async createWith(sessionIds: readonly string[]): Promise<void> {
      const project = await controller.create("New project");
      if (project === null) return;
      await controller.move(sessionIds, project.id);
      patch({ renaming: project.id });
    },
    setRenaming(id: string | null): void {
      patch({ renaming: id });
    },
    setCreating(on: boolean): void {
      patch({ creating: on });
    },
    openMover(sessionIds: readonly string[], at: { x: number; y: number }): void {
      patch({ mover: { sessionIds, at } });
    },
    closeMover(): void {
      patch({ mover: null });
    },
    askDelete(id: string | null): void {
      patch({ deleting: id });
    },
  };
  return controller;
}

/** The controller type. */
export type ProjectGroups = ReturnType<typeof createProjectGroups>;
