/**
 * The Projects overview's controller: the cards of every project the core knows
 * (manifest head + active plans, read without opening a session), each
 * project's live hub fetched lazily one at a time for its health line and detail pane, the filter, the search text,
 * the selected project, and the actions the page offers (open the handbook or a page, start a handbook, migrate,
 * new session, show the folder).
 * Depends on: coreStore (`run`, `listen`), shellStore, sessionActions, composer, toastStore, sessionSummary,
 * features/pages/model and handbookPrompts. Called by: createApp, the Projects page, the rail, the `ui/projects` route.
 * The overview is keyed by the user's projects (`groups`): a project's card is its main folder's
 * handbook (hubs stay keyed by folder: a handbook belongs to a folder); the actions take a project id; a project
 * without a main folder has no card and offers "Choose main folder…".
 * Invariants:
 * - Nothing here writes into a repository: handbook and migration requests send user messages.
 * - Hubs are fetched one at a time (a queue); a hub already loaded or loading is not asked again until `refresh`.
 * - Picking a session elsewhere closes the page (like the Tools page); a `pagesChanged` event refreshes it while open.
 */
import { toCallError } from "../protocol/client/types";
import type { PagesRequest, PagesResult, ProjectCard, ProjectHub } from "../protocol/core";
import { MIGRATE_HANDBOOK_PROMPT } from "../features/pages/model";
import {
  type HandbookPrompt,
  handbookRequestBusy,
  sendHandbookPrompt,
} from "../features/pages/handbookPrompts";
import type { Composer } from "./composer";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import { createProjectGroups } from "./projectGroups";
import type { SessionActions } from "./sessionActions";
import { summarize } from "./sessionSummary";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";

/** The status filter (the manifest's project statuses; `all` hides archived projects). */
export type StatusFilter = "all" | "active" | "maintenance" | "paused" | "archived";

/** The page's state. */
export interface ProjectsState {
  open: boolean;
  /** `null` until the first overview arrived. */
  cards: readonly ProjectCard[] | null;
  loading: boolean;
  error: string | null;
  /** How long the core took to read the heads (shown as a small note, also read by the e2e). */
  elapsedMs: number | null;
  /** Hubs by project folder; `null` while loading. */
  hubs: Readonly<Record<string, ProjectHub | null>>;
  filter: StatusFilter;
  query: string;
  /** The project (id) shown in the detail pane. */
  selected: string | null;
  /** The project whose settings replace the handbook detail. */
  settings: string | null;
}

const initial: ProjectsState = {
  open: false,
  cards: null,
  loading: false,
  error: null,
  elapsedMs: null,
  hubs: {},
  filter: "all",
  query: "",
  selected: null,
  settings: null,
};

/** Opens a session's Project tab on the hub (`path` null) or a page; registered by the Project tab. */
export type HandbookOpener = (sessionId: string, path: string | null) => void;

/** Creates the controller. */
export function createProjects(deps: {
  core: CoreController;
  shell: Shell;
  sessions: SessionActions | null;
  composer: Composer;
  toasts: Toasts;
}) {
  const { core, shell, sessions, composer, toasts } = deps;
  const store = createStore<ProjectsState>(initial);
  // the user's project groups (the session list and this overview both read them).
  const groups = createProjectGroups({ core, shell, toasts });
  const patch = (p: Partial<ProjectsState>) => {
    store.set((s) => ({ ...s, ...p }));
  };
  const run = async (request: PagesRequest): Promise<PagesResult> => {
    const result = await core.run({ type: "pages", request });
    if (result.type !== "pages") throw new Error(`unexpected ${result.type} result`);
    return result.result;
  };
  let opener: HandbookOpener | null = null;
  const queue: string[] = [];
  let draining = false;

  const drain = async (): Promise<void> => {
    if (draining) return;
    draining = true;
    try {
      for (let next = queue.shift(); next !== undefined; next = queue.shift()) {
        const project = next;
        try {
          const result = await run({ op: "projectHub", project });
          if (result.type === "projectHub")
            store.set((s) => ({ ...s, hubs: { ...s.hubs, [project]: result.project } }));
        } catch {
          // A project that went away since the overview: drop its entry; the card shows no health line.
          store.set((s) => ({
            ...s,
            hubs: Object.fromEntries(Object.entries(s.hubs).filter(([key]) => key !== project)),
          }));
        }
      }
    } finally {
      draining = false;
    }
  };
  const want = (project: string): void => {
    if (project in store.get().hubs) return;
    store.set((s) => ({ ...s, hubs: { ...s.hubs, [project]: null } }));
    queue.push(project);
    void drain();
  };

  /** A project of the groups by id. */
  const projectOf = (id: string) =>
    groups.store.get().data?.projects.find((p) => p.id === id) ?? null;
  /** A project's main folder, if it has one. */
  const folderOf = (id: string): string | null => projectOf(id)?.mainFolder ?? null;

  /** The newest open top-level session of project `id`; handbook navigation stays in the main folder. */
  const sessionIn = (id: string, mainOnly = true): string | null => {
    const folder = folderOf(id);
    const members = groups.store.get().data?.members ?? {};
    const state = core.store.get();
    const rows = Object.values(state.sessions)
      .map((e) => summarize(e, state.stuck))
      .filter(
        (s) =>
          members[s.id] === id &&
          (!mainOnly || s.cwd === folder) &&
          !s.archived &&
          s.parentId === null &&
          !s.stuck,
      )
      .sort((a, b) => b.bumpedAt - a.bumpedAt);
    return rows[0]?.id ?? null;
  };

  /** Starts a session in project `id` (its main folder) and sends `prompt` as its first message. */
  const startWith = async (project: string, prompt: string): Promise<string | null> => {
    if (composer.actions === null || folderOf(project) === null) return null;
    const id = await groups.startIn(project);
    if (id === null) return null;
    composer.setDraft(id, prompt);
    const sent = await composer.actions.send(id, "send");
    if (!sent.ok) toasts.push({ kind: "warn", bellKind: "failure", text: sent.message });
    return id;
  };

  /** The selected usable chat in the project, otherwise its newest usable chat. */
  const requestSession = (project: string): string | null => {
    const selected = shell.store.get().selected;
    const entry = selected === null ? undefined : core.store.get().sessions[selected];
    const current = entry === undefined ? null : summarize(entry, core.store.get().stuck);
    const inProject =
      current !== null &&
      groups.store.get().data?.members[current.id] === project &&
      !current.archived &&
      !current.stuck &&
      current.parentId === null;
    return inProject ? selected : sessionIn(project, false);
  };

  const requests = new Map<string, { kind: HandbookPrompt; work: Promise<void> }>();
  /** Reuses the project's current chat or starts one; concurrent clicks share that session start. */
  const sendFor = (project: string, kind: HandbookPrompt, trustedClick = false): Promise<void> => {
    const pending = requests.get(project);
    if (pending !== undefined)
      return pending.kind === kind
        ? pending.work
        : pending.work.then(() => sendFor(project, kind, trustedClick));
    const send = async () => {
      const mainFolder = folderOf(project);
      if (composer.actions === null || mainFolder === null) return;
      const id = requestSession(project) ?? (await groups.startIn(project));
      if (id === null) return;
      const folder = store.get().cards?.find((c) => c.project === mainFolder)?.folder;
      const cwd = core.store.get().sessions[id]?.session["cwd"];
      patch({ open: false });
      await sendHandbookPrompt(
        deps,
        id,
        kind,
        folder,
        cwd === mainFolder ? undefined : mainFolder,
        trustedClick,
      );
    };
    const work = send().finally(() => requests.delete(project));
    requests.set(project, { kind, work });
    return work;
  };

  const controller = {
    store,
    /** The user's project groups: projects, membership, list UI state and actions. */
    groups,
    /** Shows the page and reads the overview. */
    async open(): Promise<void> {
      patch({ open: true });
      await controller.refresh();
    },
    close(): void {
      patch({ open: false });
    },
    /** Reads the cards again; hubs are fetched again lazily (the queue). */
    async refresh(): Promise<void> {
      patch({ loading: true });
      try {
        const result = await run({ op: "overview" });
        if (result.type !== "overview") return;
        patch({ cards: result.projects, elapsedMs: result.elapsedMs, error: null, hubs: {} });
        for (const card of result.projects) if (card.state !== "none") want(card.project);
        const selected = store.get().selected;
        const folder = selected === null ? null : folderOf(selected);
        if (folder !== null) want(folder);
      } catch (thrown) {
        patch({ error: toCallError(thrown).error.message });
      } finally {
        patch({ loading: false });
      }
    },
    /** Shows project `id` in the detail pane (or closes it with `null`). */
    select(id: string | null): void {
      patch({ selected: id, settings: null });
      const folder = id === null ? null : folderOf(id);
      if (folder !== null) want(folder);
    },
    /** Opens user-owned instruction settings without starting a chat. */
    openSettings(id: string): void {
      patch({ selected: id, settings: id });
    },
    /** Returns to the selected project's detail; unsaved drafts stay in the editor controller. */
    closeSettings(): void {
      patch({ settings: null });
    },
    setFilter(filter: StatusFilter): void {
      patch({ filter });
    },
    setQuery(query: string): void {
      patch({ query });
    },
    /** Opens the handbook (or one page) of project `project` in a session's Project tab; a new one when none is open. */
    async openHandbook(project: string, path: string | null = null): Promise<void> {
      if (folderOf(project) === null) return;
      const id = sessionIn(project) ?? (await groups.startIn(project));
      if (id === null) return;
      shell.actions.selectSession(id);
      patch({ open: false });
      opener?.(id, path);
    },
    /** Whether the handbook request's target chat is working or sending. */
    handbookRequestBusy(project: string): boolean {
      const id = requestSession(project);
      return id !== null && handbookRequestBusy(composer, id);
    },
    /** Sends the handbook request in the project's current chat, or a new one. */
    async startHandbook(project: string, trustedClick = false): Promise<void> {
      await sendFor(project, "create", trustedClick);
    },
    /** Sends the first plan request separately from the project's chat draft. */
    async writePlan(project: string, trustedClick = false): Promise<void> {
      await sendFor(project, "plan", trustedClick);
    },
    /** "Migrate": a new session with the migration prompt (§11; the agent moves the files). */
    async migrate(project: string): Promise<void> {
      patch({ open: false });
      await startWith(project, MIGRATE_HANDBOOK_PROMPT);
    },
    /** A new session in project `project` (its main folder, or a folder the user picks). */
    async newSession(project: string): Promise<void> {
      const id = await groups.startIn(project);
      if (id !== null) patch({ open: false });
    },
    /** Shows the project's main folder in the file manager. */
    async reveal(project: string): Promise<void> {
      const folder = folderOf(project);
      if (folder !== null) await sessions?.revealPath(folder);
    },
    /** "Choose main folder…" (the native picker), or `folder` when a route names one. */
    async chooseFolder(project: string, folder?: string | null): Promise<void> {
      if (folder === undefined) await groups.chooseFolder(project);
      else await groups.setFolder(project, folder);
      await controller.refresh();
    },
    /** Opens a session (a "Needs you" row or a session chip). */
    openSession(sessionId: string): void {
      shell.actions.selectSession(sessionId);
      patch({ open: false });
    },
    /** Registers how a page opens in the Project tab; returns the function that removes it. */
    setOpener(fn: HandbookOpener): () => void {
      opener = fn;
      return () => {
        if (opener === fn) opener = null;
      };
    },
    /** Stops following core and shell events. */
    stop(): void {
      stopEvents();
      stopShell();
      stopGroups();
    },
  };

  let timer: ReturnType<typeof setTimeout> | null = null;
  const stopEvents = core.listen((event) => {
    // after `lagged` the core's events for this were missed: read again what is shown.
    if ((event.type !== "pagesChanged" && event.type !== "lagged") || !store.get().open) return;
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      void controller.refresh();
    }, 400);
  });
  // A main folder set, changed or cleared (or a project deleted) changes which cards exist: read them again.
  const folders = () =>
    (groups.store.get().data?.projects ?? [])
      .map((p) => p.mainFolder ?? "")
      .sort()
      .join("\n");
  let lastFolders = folders();
  const stopGroups = groups.store.subscribe(() => {
    const now = folders();
    if (now === lastFolders) return;
    lastFolders = now;
    if (store.get().open) void controller.refresh();
  });
  let selected = shell.store.get().selected;
  const stopShell = shell.store.subscribe(() => {
    const now = shell.store.get().selected;
    if (now !== selected && store.get().open) patch({ open: false });
    selected = now;
  });

  return controller;
}

/** The controller type. */
export type Projects = ReturnType<typeof createProjects>;
