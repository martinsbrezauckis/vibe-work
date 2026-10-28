/** Files state and bounded asynchronous reads. Depends on the typed core client; used by FilesPanel and its views. */
import type {
  BrowserStatus,
  ExplorerEntry,
  ExplorerHit,
  ExplorerRequest,
  ExplorerResult,
  ExplorerVersion,
} from "../../protocol/core";
import type { CoreController } from "../../state/coreStore";
import { createStore } from "../../state/createStore";

type FolderPage = { entries: ExplorerEntry[]; ignored: string[]; next: number | null };
export interface ExplorerState {
  folder: string | null;
  view: "files" | "history";
  folders: Readonly<Record<string, FolderPage>>;
  expanded: readonly string[];
  loadingFolders: readonly string[];
  status: BrowserStatus | null;
  statusLoading: boolean;
  historyNote: string | null;
  commitCount: number | null;
  latest: ExplorerVersion | null;
  changedOnly: boolean;
  showIgnored: boolean;
  paths: readonly string[];
  indexing: boolean;
  indexLimited: boolean;
  query: string;
  searching: boolean;
  names: readonly string[];
  hits: readonly ExplorerHit[];
  searchVersions: readonly ExplorerVersion[];
  searchLimited: boolean;
  versions: readonly ExplorerVersion[];
  cursor: string | null;
  historyLoading: boolean;
  historyLoaded: boolean;
  version: ExplorerVersion | null;
  versionFiles: readonly string[];
  versionLoading: boolean;
  versionLimited: boolean;
  historyScroll: number;
  /** History's "Changes since the last commit" disclosure is open. */
  pendingOpen: boolean;
  /** History shows only the commits that changed this file (History of this file). */
  historyPath: string | null;
  /** The row Reveal in Files selected; the tree focuses it once it is shown. */
  revealed: string | null;
  /** Search in this folder: results only inside this folder. */
  scope: string | null;
  error: string | null;
}
const initial: ExplorerState = {
  folder: null,
  view: "files",
  folders: {},
  expanded: [],
  loadingFolders: [],
  status: null,
  statusLoading: false,
  historyNote: null,
  commitCount: null,
  latest: null,
  changedOnly: false,
  showIgnored: false,
  paths: [],
  indexing: false,
  indexLimited: false,
  query: "",
  searching: false,
  names: [],
  hits: [],
  searchVersions: [],
  searchLimited: false,
  versions: [],
  cursor: null,
  historyLoading: false,
  historyLoaded: false,
  version: null,
  versionFiles: [],
  versionLoading: false,
  versionLimited: false,
  historyScroll: 0,
  pendingOpen: false,
  historyPath: null,
  revealed: null,
  scope: null,
  error: null,
};
/** Name search shows at most this many files ("First 200 files"). */
export const NAME_LIMIT = 200;
/** Search in this folder: whether `path` lies inside `scope` (`null`: the whole folder). */
const inside = (scope: string | null, path: string) =>
  scope === null || path.startsWith(`${scope}/`);
/** Name matches inside the scope, so the first 200 are counted there and not taken by files outside it. */
function scopedNames(paths: readonly string[], query: string, scope: string | null): string[] {
  return nameMatches(scope === null ? paths : paths.filter((p) => inside(scope, p)), query);
}
/**
 * The name and text results the search shows: with Search in this folder (`scope`) only those inside that folder.
 * The list, its "First 200 files" note, Enter and the Debug API's `openFirst` all use this, so Enter opens the
 * first result the list shows.
 */
export function shownResults(s: Pick<ExplorerState, "scope" | "names" | "hits">): {
  names: readonly string[];
  hits: readonly ExplorerHit[];
} {
  const scope = s.scope;
  if (scope === null) return { names: s.names, hits: s.hits };
  return {
    names: s.names.filter((path) => inside(scope, path)),
    hits: s.hits.filter((hit) => inside(scope, hit.path)),
  };
}
/** Fuzzy path search keeps at most two hundred matches, preferring adjacent characters. */
export function nameMatches(paths: readonly string[], query: string): string[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [];
  const scored: { path: string; score: number }[] = [];
  for (const path of paths) {
    const lower = path.toLowerCase();
    let at = -1,
      gaps = 0,
      found = true;
    for (const char of needle) {
      const next = lower.indexOf(char, at + 1);
      if (next < 0) {
        found = false;
        break;
      }
      gaps += next - at - 1;
      at = next;
    }
    if (found) scored.push({ path, score: lower.includes(needle) ? -1000 + path.length : gaps });
  }
  return scored
    .sort((a, b) => a.score - b.score || a.path.localeCompare(b.path))
    .slice(0, NAME_LIMIT)
    .map((s) => s.path);
}
/** A controller survives switching editor tabs; every answer is scoped to its folder and generation. */
export function createExplorer(core: CoreController) {
  const store = createStore<ExplorerState>(initial);
  let generation = 0,
    searchGeneration = 0,
    versionGeneration = 0;
  let autoFilterPending = true;
  let indexCall = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const run = async (folder: string, request: ExplorerRequest): Promise<ExplorerResult> => {
    const answer = await core.run(
      {
        type: "gitBrowser",
        request: { op: "explore", folder, request },
      },
      ["revert", "enableHistory"].includes(request.op) ? undefined : { origin: "window" },
    );
    if (answer.type !== "gitBrowser" || answer.result.kind !== "explorer")
      throw new Error("Files response unavailable");
    return answer.result.result;
  };
  const failure = (gen: number) => {
    if (gen === generation)
      store.set((s) => ({
        ...s,
        error:
          "This part of the folder could not be read. Try Refresh; the other views are still available.",
      }));
  };
  async function loadFolder(path: string, append = false) {
    const state = store.get(),
      folder = state.folder,
      gen = generation;
    if (!folder || state.loadingFolders.includes(path)) return;
    const offset = append ? state.folders[path]?.next : 0;
    if (offset == null) return;
    store.set((s) => ({ ...s, loadingFolders: [...s.loadingFolders, path] }));
    try {
      const result = await run(folder, {
        op: "folder",
        path,
        showIgnored: state.showIgnored,
        offset,
      });
      if (gen !== generation || result.kind !== "folder") return;
      store.set((s) => ({
        ...s,
        folders: {
          ...s.folders,
          [path]: {
            entries: append
              ? [...(s.folders[path]?.entries ?? []), ...result.entries]
              : result.entries,
            ignored: result.ignored,
            next: result.next,
          },
        },
        status: result.status ?? s.status,
      }));
    } catch {
      failure(gen);
    } finally {
      if (gen === generation)
        store.set((s) => ({ ...s, loadingFolders: s.loadingFolders.filter((p) => p !== path) }));
    }
  }
  async function history(more = false) {
    const s = store.get(),
      folder = s.folder,
      gen = generation;
    if (!folder || s.historyLoading || (more && !s.cursor)) return;
    store.set((s) => ({ ...s, historyLoading: true }));
    try {
      const result = await run(folder, {
        op: "history",
        cursor: more ? s.cursor : null,
        query: "",
        ...(s.historyPath === null ? {} : { path: s.historyPath }),
      });
      if (gen !== generation || result.kind !== "history") return;
      store.set((s) => ({
        ...s,
        versions: more ? [...s.versions, ...result.versions] : result.versions,
        cursor: result.cursor,
        historyLoaded: true,
      }));
    } catch {
      failure(gen);
    } finally {
      if (gen === generation) store.set((s) => ({ ...s, historyLoading: false }));
    }
  }
  async function buildIndex(folder: string, gen: number) {
    const call = ++indexCall;
    try {
      const result = await run(folder, { op: "index" });
      if (gen !== generation || call !== indexCall || result.kind !== "index") return;
      const autoFilter = autoFilterPending && !store.get().statusLoading;
      if (autoFilter) autoFilterPending = false;
      store.set((s) => ({
        ...s,
        paths: result.paths,
        indexLimited: result.truncated,
        changedOnly:
          autoFilter && result.paths.length > 5000 && s.status !== null ? true : s.changedOnly,
        indexing: false,
        names: scopedNames(result.paths, s.query, s.scope),
      }));
    } catch {
      failure(gen);
      if (gen === generation) store.set((s) => ({ ...s, indexing: false }));
    }
  }
  async function loadStatus() {
    const folder = store.get().folder,
      gen = generation;
    if (!folder) return;
    store.set((s) => ({ ...s, statusLoading: true }));
    try {
      const result = await run(folder, { op: "status" });
      if (gen === generation && result.kind === "status")
        store.set((s) => ({
          ...s,
          status: result.status,
          historyNote: result.note,
          commitCount: result.commits,
          latest: result.latest,
        }));
    } catch {
      if (gen === generation)
        store.set((s) => ({
          ...s,
          historyNote: "Files still work; history needs Git. Check Git and Refresh.",
        }));
    } finally {
      if (gen === generation)
        store.set((s) => {
          const apply = autoFilterPending && !s.indexing;
          if (apply) autoFilterPending = false;
          return {
            ...s,
            statusLoading: false,
            changedOnly: apply ? s.status !== null && s.paths.length > 5000 : s.changedOnly,
          };
        });
    }
  }
  const controller = {
    store,
    run,
    choose(folder: string | null) {
      if (store.get().folder === folder) return;
      const old = store.get().folder;
      if (old) void run(old, { op: "cancel" }).catch(() => undefined);
      clearTimeout(timer);
      searchGeneration++;
      versionGeneration++;
      const gen = ++generation;
      autoFilterPending = true;
      const view = store.get().view;
      store.set({ ...initial, view, folder, indexing: folder !== null });
      if (folder) {
        void loadFolder("");
        void loadStatus();
        void buildIndex(folder, gen);
        if (view === "history") void history();
      }
    },
    loadFolder,
    history,
    setView(view: "files" | "history") {
      store.set((s) => ({ ...s, view, version: null }));
      if (view === "history" && !store.get().historyLoaded) void history();
    },
    toggle(path: string) {
      const open = store.get().expanded.includes(path);
      store.set((s) => ({
        ...s,
        expanded: open ? s.expanded.filter((p) => p !== path) : [...s.expanded, path],
      }));
      if (!open && !store.get().folders[path]) void loadFolder(path);
    },
    /** Close all folders. */
    collapseAll() {
      store.set((s) => ({ ...s, expanded: [] }));
    },
    changedOnly(value: boolean) {
      autoFilterPending = false;
      store.set((s) => ({ ...s, changedOnly: value }));
    },
    showIgnored(value: boolean) {
      const before = store.get(),
        folder = before.folder;
      if (!folder) return;
      const gen = ++generation;
      store.set({
        ...initial,
        folder,
        showIgnored: value,
        changedOnly: before.changedOnly,
        expanded: before.expanded,
        view: before.view,
        pendingOpen: before.pendingOpen,
        indexing: true,
      });
      void loadFolder("");
      void loadStatus();
      void buildIndex(folder, gen);
      for (const path of before.expanded) void loadFolder(path);
      if (before.view === "history") void history();
    },
    query(query: string) {
      clearTimeout(timer);
      const call = ++searchGeneration,
        gen = generation,
        folder = store.get().folder;
      if (folder) void run(folder, { op: "cancel" }).catch(() => undefined);
      store.set((s) => ({
        ...s,
        query,
        names: scopedNames(s.paths, query, s.scope),
        hits: [],
        searchVersions: [],
        searching: !!query.trim(),
        searchLimited: false,
      }));
      if (!folder || !query.trim()) return;
      // Search in this folder: the core searches only there, so its 500-hit cap counts inside the folder.
      const scope = store.get().scope;
      timer = setTimeout(() => {
        let pending = 2;
        const alive = () => call === searchGeneration && gen === generation;
        const finish = () => {
          pending--;
          if (alive() && pending === 0) store.set((s) => ({ ...s, searching: false }));
        };
        void run(folder, { op: "search", query, ...(scope === null ? {} : { path: scope }) })
          .then((result) => {
            if (alive() && result.kind === "search")
              store.set((s) => ({ ...s, hits: result.hits, searchLimited: result.truncated }));
          })
          .catch(() => {
            if (alive()) store.set((s) => ({ ...s, searchLimited: true }));
          })
          .finally(finish);
        void run(folder, { op: "history", cursor: null, query })
          .then((result) => {
            if (alive() && result.kind === "history")
              store.set((s) => ({ ...s, searchVersions: result.versions }));
          })
          .catch(() => undefined)
          .finally(finish);
      }, 150);
    },
    async openVersion(version: ExplorerVersion) {
      const folder = store.get().folder,
        gen = generation,
        call = ++versionGeneration;
      if (!folder) return;
      store.set((s) => ({
        ...s,
        version,
        versionFiles: [],
        versionLoading: true,
        versionLimited: false,
      }));
      try {
        const result = await run(folder, { op: "version", id: version.id });
        if (gen === generation && call === versionGeneration && result.kind === "version")
          store.set((s) => ({
            ...s,
            versionFiles: result.files,
            versionLimited: result.truncated,
          }));
      } catch {
        failure(gen);
      } finally {
        if (gen === generation && call === versionGeneration)
          store.set((s) => ({ ...s, versionLoading: false }));
      }
    },
    /** A commit found by the search opens like its row's click: the search clears and History shows it. */
    openFound(version: ExplorerVersion) {
      controller.query("");
      controller.setView("history");
      return controller.openVersion(version);
    },
    /** History of this file (`path`), or of the whole folder again (`null`). */
    historyOf(path: string | null) {
      versionGeneration++;
      controller.query("");
      store.set((s) => ({
        ...s,
        view: "history",
        historyPath: path,
        version: null,
        versions: [],
        cursor: null,
        historyLoaded: false,
        historyScroll: 0,
      }));
      void history();
    },
    /** Reveal in Files: the Files view with every folder above `path` open and its row selected. */
    reveal(path: string) {
      controller.query("");
      const parents = path
        .split("/")
        .slice(0, -1)
        .map((_, index, parts) => parts.slice(0, index + 1).join("/"));
      store.set((s) => ({
        ...s,
        view: "files",
        version: null,
        changedOnly: s.changedOnly && (s.status?.files.some((f) => f.path === path) ?? false),
        expanded: [...s.expanded, ...parents.filter((p) => !s.expanded.includes(p))],
        revealed: path,
      }));
      for (const parent of parents) if (!store.get().folders[parent]) void loadFolder(parent);
    },
    /** The tree took the revealed row. */
    revealDone() {
      store.set((s) => (s.revealed === null ? s : { ...s, revealed: null }));
    },
    /** Search in this folder (`null`: the whole folder again); the search field gets the focus. */
    searchIn(scope: string | null) {
      store.set((s) => ({ ...s, scope }));
      // The names and the core's text search run again for the new folder.
      controller.query(store.get().query);
    },
    /** Escape in the search field: the text and Search in this folder go, as one search (emptying the field keeps the folder). */
    clearSearch() {
      controller.query("");
      store.set((s) => ({ ...s, scope: null }));
    },
    /** One file as it was in commit `id`: its text, or the reason it cannot be shown. */
    async readAt(id: string, path: string): Promise<{ text: string | null; note: string | null }> {
      const folder = store.get().folder;
      if (!folder) return { text: null, note: "Choose a folder first." };
      const result = await run(folder, { op: "readAt", id, path });
      return result.kind === "text"
        ? { text: result.text, note: result.note }
        : { text: null, note: "This commit could not be read. Try again." };
    },
    /** Opens or closes History's "Changes since the last commit" disclosure. */
    setPending(open: boolean) {
      store.set((s) => ({ ...s, pendingOpen: open }));
    },
    back() {
      versionGeneration++;
      store.set((s) => ({ ...s, version: null, versionLoading: false }));
    },
    scroll(value: number) {
      store.set((s) => ({ ...s, historyScroll: value }));
    },
    refresh() {
      store.set((s) => ({ ...s, error: null }));
      void loadFolder("");
      void loadStatus();
      for (const path of store.get().expanded) void loadFolder(path);
      const folder = store.get().folder;
      if (folder) {
        store.set((s) => ({ ...s, indexing: true }));
        void buildIndex(folder, generation);
      }
      if (store.get().historyLoaded) void history();
    },
    stop() {
      clearTimeout(timer);
      generation++;
      searchGeneration++;
      const folder = store.get().folder;
      if (folder) void run(folder, { op: "cancel" }).catch(() => undefined);
    },
  };
  return controller;
}
export type Explorer = ReturnType<typeof createExplorer>;
