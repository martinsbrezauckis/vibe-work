/**
 * Files in the chat: which path texts in a session's transcript are clickable (batched `files resolve`
 * through the core, cached per session), and the File tab per session (the shown file, a short recent list, its
 * content, live reload, the isolated viewer for HTML pages and PDFs).
 *
 * Depends on: coreStore (commands, events), createStore, protocol types. Called by: createApp, the transcript's
 * `FileRef` and step views, the File tab, the `ui/files` route.
 * Invariants (the "preview bugs out" class):
 * - Every read carries a generation number per session; an answer for an older generation is dropped, so rapid
 *   switching never shows file A under B's name.
 * - The last good content stays on screen while a reload runs; a deleted file keeps it with a "deleted" notice.
 * - The core decides what is clickable; a refused path stays plain text and nothing tells why (no oracle).
 * - Actions built for the Debug API (`actions("debug-api")`) send every command with that origin.
 */
import type {
  CoreEvent,
  FileContent,
  FileKind,
  FileChoice,
  FilesRequest,
  FilesResult,
  Rect,
  ViewerView,
} from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { DropReport } from "../features/files/drop";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import { pruneSessions } from "./prune";
import { beginFileEdit, normalizeEditText, observeFileEdit, type FileEdit } from "./fileEdits";

/** Maximum inline images kept ready for display. */
const IMAGE_CACHE = 30;

/** What the core said about one path text. */
export type RefState =
  | { status: "pending" }
  | { status: "no" }
  | { status: "choices"; choices: readonly FileChoice[]; total: number }
  | { status: "ok"; file: string; display: string; kind: FileKind | null; size: number | null };

/** A file the tab shows or showed. */
export interface Shown {
  file: string;
  display: string;
  line: number | null;
}

/** One session's File tab. */
export interface FileTab {
  shown: Shown | null;
  /** Newest first, at most `RECENT`. */
  recent: readonly Shown[];
  status: "idle" | "loading" | "ready" | "gone" | "error";
  error: string | null;
  editNotice: string | null;
  /** The last content read for `shown` (kept while reloading and after a delete). */
  content: FileContent | null;
  /** `rendered` (Markdown rendered, HTML/PDF in the viewer, images) or `source` (the text). */
  mode: "rendered" | "source";
  viewer: ViewerView | null;
  /** Bumped by every open and reload; answers for an older value are dropped. */
  gen: number;
}

/** Everything the controller keeps. */
export interface FilesState {
  refs: Readonly<Record<string, Readonly<Record<string, RefState>>>>;
  tabs: Readonly<Record<string, FileTab>>;
  /** The last drop on each session's composer (what was attached, what was refused and why). */
  drops: Readonly<Record<string, DropReport>>;
  /** Open buffers retain their own baseline even when the disk preview advances. */
  edits: Readonly<Record<string, FileEdit>>;
}

/** Most entries in a tab's recent list. */
export const RECENT = 12;
/** How often the shown file is checked while the tab is visible (ms). */
export const POLL_MS = 1500;
/** When paths whose lookup failed are asked again (ms). */
const RETRY_MS = 3000;
/** Most paths sent in one `resolve` (the core's limit is 200). */
const BATCH = 100;

const EMPTY_TAB: FileTab = {
  shown: null,
  recent: [],
  status: "idle",
  error: null,
  editNotice: null,
  content: null,
  mode: "rendered",
  viewer: null,
  gen: 0,
};

/** The tab of `sessionId` in `state` (an empty one when it has none). */
export function tabOf(state: FilesState, sessionId: string): FileTab {
  return state.tabs[sessionId] ?? EMPTY_TAB;
}

/** True for kinds the isolated viewer shows. */
export function viewerKind(kind: FileKind | null | undefined): boolean {
  return kind === "html" || kind === "pdf";
}

/** Creates the controller. */
export function createFiles(core: CoreController) {
  const store = createStore<FilesState>({ refs: {}, tabs: {}, drops: {}, edits: {} });
  const queue = new Map<string, Set<string>>();
  let timer: ReturnType<typeof setTimeout> | null = null;

  const run = async (origin: "ui" | "debug-api", request: FilesRequest): Promise<FilesResult> => {
    const result = await core.run(
      { type: "files", request },
      origin === "debug-api" ? { origin } : undefined,
    );
    if (result.type !== "files") throw new Error(`unexpected ${result.type} result`);
    return result.result;
  };
  const setRefs = (sessionId: string, update: Record<string, RefState>) => {
    store.set((s) => ({
      ...s,
      refs: { ...s.refs, [sessionId]: { ...(s.refs[sessionId] ?? {}), ...update } },
    }));
  };
  const setTab = (sessionId: string, update: (tab: FileTab) => FileTab) => {
    store.set((s) => ({ ...s, tabs: { ...s.tabs, [sessionId]: update(tabOf(s, sessionId)) } }));
  };
  const edit = {
    begin(sessionId: string): boolean {
      const tab = tabOf(store.get(), sessionId);
      if (tab.status !== "ready" || tab.content === null) return false;
      if (store.get().edits[sessionId] !== undefined) return true;
      const draft = beginFileEdit(tab.content);
      if (draft === null) return false;
      store.set((s) => ({ ...s, edits: { ...s.edits, [sessionId]: draft } }));
      return true;
    },
    update(sessionId: string, patch: Partial<Omit<FileEdit, "id" | "base" | "disk">>): void {
      store.set((s) => {
        const draft = s.edits[sessionId];
        return draft === undefined
          ? s
          : {
              ...s,
              edits: {
                ...s.edits,
                [sessionId]: {
                  ...draft,
                  ...patch,
                  ...(patch.text === undefined ? {} : { text: normalizeEditText(patch.text) }),
                },
              },
            };
      });
    },
    cancel(sessionId: string): void {
      store.set((s) => {
        const next = Object.fromEntries(Object.entries(s.edits).filter(([id]) => id !== sessionId));
        return { ...s, edits: next };
      });
    },
    /** Explicitly discard edits after a successful fresh read. */
    takeDisk(sessionId: string): boolean {
      const current = store.get().edits[sessionId];
      if (current === undefined || current.saving) return false;
      const tab = tabOf(store.get(), sessionId);
      const shown = tab.shown?.file === current.base.file;
      if (shown && (tab.status !== "ready" || tab.content === null)) return false;
      const draft = beginFileEdit(shown ? (tab.content ?? current.disk) : current.disk);
      if (draft === null) return false;
      store.set((s) => ({ ...s, edits: { ...s.edits, [sessionId]: draft } }));
      return true;
    },
  };

  const flush = async (): Promise<void> => {
    timer = null;
    const work = [...queue.entries()];
    queue.clear();
    for (const [sessionId, set] of work) {
      const paths = [...set];
      for (let i = 0; i < paths.length; i += BATCH) {
        const chunk = paths.slice(i, i + BATCH);
        try {
          const result = await run("ui", { op: "resolve", sessionId, paths: chunk });
          if (result.kind !== "resolved") continue;
          const update: Record<string, RefState> = {};
          for (const ref of result.refs) {
            update[ref.text] =
              ref.ok && ref.file !== null && ref.display !== null
                ? {
                    status: "ok",
                    file: ref.file,
                    display: ref.display,
                    kind: ref.kind,
                    size: ref.size,
                  }
                : ref.choices !== undefined && ref.choices.length > 1
                  ? {
                      status: "choices",
                      choices: ref.choices,
                      total: ref.total ?? ref.choices.length,
                    }
                  : { status: "no" };
          }
          setRefs(sessionId, update);
        } catch {
          // The core could not answer (a session it does not know yet, a restart): plain for now, and asked again
          // after RETRY_MS (a failed lookup must not leave references dead for the rest of the session).
          setRefs(sessionId, Object.fromEntries(chunk.map((p) => [p, { status: "no" }])));
          setTimeout(() => {
            forget(sessionId, chunk);
          }, RETRY_MS);
        }
      }
    }
  };

  /** Drops the answers for `paths` (they are asked again by the components that show them). */
  const forget = (sessionId: string, paths: readonly string[]): void => {
    store.set((s) => {
      const refs = s.refs[sessionId];
      if (refs === undefined) return s;
      const kept = Object.fromEntries(Object.entries(refs).filter(([p]) => !paths.includes(p)));
      return { ...s, refs: { ...s.refs, [sessionId]: kept } };
    });
  };

  /** Asks the core about `text` once (batched); components call this for every path they show. */
  const want = (sessionId: string, text: string): void => {
    if (store.get().refs[sessionId]?.[text] !== undefined) return;
    setRefs(sessionId, { [text]: { status: "pending" } });
    let set = queue.get(sessionId);
    if (set === undefined) queue.set(sessionId, (set = new Set()));
    set.add(text);
    timer ??= setTimeout(() => void flush(), 30);
  };

  /**
   * an image file of the session as a `data:` URL for an inline picture in the transcript (`![..](path)`
   * in an answer), or `null` (not an image, too large, unreadable). Cached per session and handle (at most
   * `IMAGE_CACHE` entries; the oldest goes first).
   */
  const images = new Map<string, Promise<string | null>>();
  const image = (sessionId: string, file: string): Promise<string | null> => {
    const key = `${sessionId}\u0000${file}`;
    let found = images.get(key);
    if (found === undefined) {
      found = run("ui", { op: "read", sessionId, file })
        .then((r) =>
          r.kind === "content" && r.content.kind === "image" ? r.content.dataUrl : null,
        )
        .catch(() => null);
      images.set(key, found);
      if (images.size > IMAGE_CACHE) {
        const oldest = images.keys().next().value;
        if (oldest !== undefined) images.delete(oldest);
      }
    }
    return found;
  };

  /** Forgets the "not a file" answers of a session (files may exist now). */
  const invalidate = (sessionId: string): void => {
    store.set((s) => {
      const refs = s.refs[sessionId];
      if (refs === undefined) return s;
      const kept = Object.fromEntries(Object.entries(refs).filter(([, r]) => r.status !== "no"));
      return { ...s, refs: { ...s.refs, [sessionId]: kept } };
    });
  };

  /** Reads the shown file again (generation-checked). `keep`: the old content stays while it loads. */
  const load = async (origin: "ui" | "debug-api", sessionId: string): Promise<void> => {
    const before = tabOf(store.get(), sessionId);
    const target = before.shown;
    if (target === null) return;
    const gen = before.gen + 1;
    setTab(sessionId, (t) => ({ ...t, gen, status: "loading", error: null }));
    const current = () => store.get().tabs[sessionId]?.gen === gen;
    try {
      const result = await run(origin, { op: "read", sessionId, file: target.file });
      if (!current() || result.kind !== "content") return;
      store.set((s) => {
        const draft = s.edits[sessionId];
        return {
          ...s,
          tabs: {
            ...s.tabs,
            [sessionId]: { ...tabOf(s, sessionId), status: "ready", content: result.content },
          },
          edits:
            draft === undefined
              ? s.edits
              : { ...s.edits, [sessionId]: observeFileEdit(draft, result.content) },
        };
      });
      if (viewerKind(result.content.kind) && store.get().tabs[sessionId]?.mode === "rendered") {
        const viewed = await run(origin, { op: "view", sessionId, file: target.file });
        if (current() && viewed.kind === "viewer")
          setTab(sessionId, (t) => ({ ...t, viewer: viewed.viewer }));
      }
    } catch (thrown) {
      if (!current()) return;
      const error = toCallError(thrown).error;
      setTab(sessionId, (t) => ({
        ...t,
        status: error.code === "notFound" ? "gone" : "error",
        error: error.code === "notFound" ? null : error.message,
      }));
    }
  };

  /** Refresh the edited file even when a different file is waiting to be shown after Save or Cancel. */
  const refreshDraft = async (origin: "ui" | "debug-api", sessionId: string): Promise<boolean> => {
    const draft = store.get().edits[sessionId];
    if (draft === undefined) return false;
    const gen = tabOf(store.get(), sessionId).gen;
    try {
      const result = await run(origin, { op: "read", sessionId, file: draft.base.file });
      if (
        result.kind !== "content" ||
        result.content.file !== draft.base.file ||
        store.get().edits[sessionId]?.id !== draft.id
      )
        return false;
      store.set((s) => {
        const tab = tabOf(s, sessionId);
        return {
          ...s,
          tabs:
            tab.shown?.file === draft.base.file && tab.gen === gen
              ? {
                  ...s.tabs,
                  [sessionId]: { ...tab, content: result.content, status: "ready", error: null },
                }
              : s.tabs,
          edits: {
            ...s.edits,
            [sessionId]: observeFileEdit(s.edits[sessionId] ?? draft, result.content),
          },
        };
      });
      return true;
    } catch (thrown) {
      if (store.get().edits[sessionId]?.id === draft.id)
        edit.update(sessionId, { problem: toCallError(thrown).error.message });
      return false;
    }
  };

  /** One origin's actions (the window's own clicks, or the `ui/files` route). */
  const actions = (origin: "ui" | "debug-api") => ({
    /** Shows a resolved file in `sessionId`'s tab (and puts it first in the recent list). */
    async show(sessionId: string, shown: Shown): Promise<void> {
      const draft = store.get().edits[sessionId];
      if (
        tabOf(store.get(), sessionId).shown?.file !== shown.file &&
        draft !== undefined &&
        !draft.saving &&
        draft.text === draft.base.text
      )
        edit.cancel(sessionId);
      setTab(sessionId, (t) => ({
        ...t,
        shown,
        recent: [shown, ...t.recent.filter((r) => r.file !== shown.file)].slice(0, RECENT),
        content: t.shown?.file === shown.file ? t.content : null,
        viewer: t.shown?.file === shown.file ? t.viewer : null,
        mode: t.shown?.file === shown.file ? t.mode : "rendered",
        editNotice: null,
      }));
      await load(origin, sessionId);
      if (store.get().edits[sessionId]?.base.file !== shown.file)
        await refreshDraft(origin, sessionId);
    },
    /** Open from a file link, then begin editing only that read's complete text. */
    async showForEdit(sessionId: string, shown: Shown): Promise<void> {
      const loading = this.show(sessionId, shown);
      const gen = tabOf(store.get(), sessionId).gen;
      await loading;
      const tab = tabOf(store.get(), sessionId);
      if (tab.gen !== gen || tab.shown?.file !== shown.file || tab.status !== "ready") return;
      const draft = store.get().edits[sessionId];
      const reason =
        draft !== undefined && draft.base.file !== shown.file
          ? "Save or cancel your current edit first."
          : tab.content?.tooLarge === true
            ? "The file is too large to edit here."
            : tab.content?.truncated === true
              ? "Only the start of the file was read."
              : null;
      if (reason !== null || !edit.begin(sessionId))
        setTab(sessionId, (t) => ({
          ...t,
          editNotice: `The editor did not open. ${reason ?? "This file has no editable text."}`,
        }));
    },
    /** Resolve for a link menu with this origin, keeping choices available to the caller. */
    async resolveRef(sessionId: string, text: string): Promise<RefState> {
      const result = await run(origin, { op: "resolve", sessionId, paths: [text] });
      const ref = result.kind === "resolved" ? result.refs[0] : undefined;
      const state: RefState =
        ref?.ok === true && ref.file !== null && ref.display !== null
          ? { status: "ok", file: ref.file, display: ref.display, kind: ref.kind, size: ref.size }
          : ref?.choices !== undefined && ref.choices.length > 1
            ? { status: "choices", choices: ref.choices, total: ref.total ?? ref.choices.length }
            : { status: "no" };
      setRefs(sessionId, { [text]: state });
      return state;
    },
    /** Resolves `text` with this origin's scope and shows it; false when it is not a file the app may show. */
    async openText(sessionId: string, text: string, line: number | null = null): Promise<boolean> {
      const result = await run(origin, { op: "resolve", sessionId, paths: [text] });
      const ref = result.kind === "resolved" ? result.refs[0] : undefined;
      if (ref?.ok !== true || ref.file === null || ref.display === null) return false;
      await this.show(sessionId, { file: ref.file, display: ref.display, line });
      return true;
    },
    /** Reads the shown file again. */
    async reload(sessionId: string): Promise<void> {
      await load(origin, sessionId);
      const draft = store.get().edits[sessionId];
      if (draft !== undefined && draft.base.file !== tabOf(store.get(), sessionId).shown?.file)
        await refreshDraft(origin, sessionId);
    },
    /** Explicit discard uses a fresh read of the edited file, never the pending preview. */
    async reloadEdit(sessionId: string): Promise<boolean> {
      const draft = store.get().edits[sessionId];
      if (draft === undefined || draft.saving) return false;
      return (await refreshDraft(origin, sessionId)) && edit.takeDisk(sessionId);
    },
    /** Rendered view or source text. */
    async setMode(sessionId: string, mode: FileTab["mode"]): Promise<void> {
      setTab(sessionId, (t) => ({ ...t, mode }));
      if (mode === "rendered") await load(origin, sessionId);
    },
    /**
     * Closes the shown file only: it leaves the tab's file list and the next file in the list is shown. The tab is
     * empty only when it was the last file (emptying the whole tab looked as if every file in the list was closed).
     */
    close(sessionId: string): void {
      const draft = store.get().edits[sessionId];
      if (draft?.saving || (draft !== undefined && draft.text !== draft.base.text))
        throw new Error("Save or Cancel your edit before closing the file.");
      edit.cancel(sessionId);
      const before = tabOf(store.get(), sessionId);
      const rest = before.recent.filter((r) => r.file !== before.shown?.file);
      setTab(sessionId, (t) => ({ ...EMPTY_TAB, recent: rest, gen: t.gen + 1 }));
      const next = rest[0];
      if (next !== undefined) void this.show(sessionId, { ...next, line: null });
    },
    /**
     * Saves `text` as the shown file's whole content. The core refuses when the file changed
     * on disk since the tab read it (`stale`), keeps line endings and a byte-order mark, and writes the file whole.
     * Returns `saved` (the tab reloads), `stale`, or the refusal's message.
     * `base` is the file and fingerprint the edit STARTED from (the editor pins them when it opens). Without it the
     * shown file's current fingerprint is used; with it a newer disk version (the agent's change, which the tab's
     * poll loads meanwhile) can never be overwritten by an older draft: the core answers `stale`.
     */
    async write(
      sessionId: string,
      text: string,
      base?: { file: string; fingerprint: string },
    ): Promise<{ kind: "saved" } | { kind: "stale" } | { kind: "error"; message: string }> {
      const tab = store.get().tabs[sessionId];
      const draft = store.get().edits[sessionId];
      const file = base?.file ?? tab?.shown?.file;
      const fingerprint = base?.fingerprint ?? tab?.content?.fingerprint;
      if (file === undefined || fingerprint === undefined)
        return { kind: "error", message: "No file is shown." };
      if (draft !== undefined && draft.base.file !== file)
        return { kind: "error", message: "The editor's file is no longer shown." };
      try {
        await run(origin, { op: "write", sessionId, file, text, fingerprint });
      } catch (thrown) {
        const error = toCallError(thrown).error;
        return error.code === "stale"
          ? { kind: "stale" }
          : { kind: "error", message: error.message };
      }
      await load(origin, sessionId);
      return { kind: "saved" };
    },
    /** Opens the shown file with its default app, or shows it in its folder (the core asks where it must). */
    async external(
      sessionId: string,
      what: "open" | "reveal",
      handle?: string,
    ): Promise<string | null> {
      const file = handle ?? store.get().tabs[sessionId]?.shown?.file;
      if (file === undefined) return null;
      try {
        await run(origin, { op: what, sessionId, file });
        return null;
      } catch (thrown) {
        const error = toCallError(thrown).error;
        return error.code === "confirmationDeclined" ? null : error.message;
      }
    },
  });

  /** Checks the shown file; reloads when it changed, marks it gone when it was deleted. */
  const poll = async (sessionId: string): Promise<void> => {
    const tab = store.get().tabs[sessionId];
    if (tab?.shown == null || tab.status === "loading") return;
    const gen = tab.gen;
    try {
      const result = await run("ui", { op: "stat", sessionId, file: tab.shown.file });
      if (result.kind !== "stat" || store.get().tabs[sessionId]?.gen !== gen) return;
      if (!result.exists) {
        if (tab.status !== "gone") setTab(sessionId, (t) => ({ ...t, status: "gone" }));
      } else if (result.fingerprint !== tab.content?.fingerprint || tab.status === "gone") {
        await load("ui", sessionId);
      }
    } catch {
      // The next tick tries again.
    }
    const draft = store.get().edits[sessionId];
    if (draft !== undefined && draft.base.file !== tabOf(store.get(), sessionId).shown?.file)
      await refreshDraft("ui", sessionId);
  };

  /** Where the viewer goes (null: hidden). Serialized; only the newest placement is sent. */
  let placed: { sessionId: string; rect: Rect } | null = null;
  let sent: string | undefined;
  let sending = false;
  const layout = async (next: { sessionId: string; rect: Rect } | null): Promise<void> => {
    placed = next;
    if (sending) return;
    sending = true;
    try {
      while (sent !== JSON.stringify(placed)) {
        const target = placed;
        sent = JSON.stringify(target);
        await run(
          "ui",
          target === null
            ? { op: "layout" }
            : { op: "layout", sessionId: target.sessionId, rect: target.rect },
        ).catch(() => undefined);
      }
    } finally {
      sending = false;
    }
  };

  // a deleted session's entries go with it.
  core.onSessionsGone((ids) => {
    store.set((s) => pruneSessions(s, ["refs", "tabs", "drops", "edits"], ids));
  });
  /** Reads `sessionId`'s viewer state again (after `filesChanged`, or a missed one). */
  const readViewer = (sessionId: string): void => {
    void run("ui", { op: "viewer", sessionId })
      .then((r) => {
        if (r.kind === "viewer") setTab(sessionId, (t) => ({ ...t, viewer: r.viewer }));
      })
      .catch(() => undefined);
  };
  const stop = core.listen((event: CoreEvent) => {
    if (event.type === "changesUpdated") {
      invalidate(event.sessionId);
      void poll(event.sessionId);
    } else if (event.type === "filesChanged" && event.sessionId !== null) {
      readViewer(event.sessionId);
    } else if (event.type === "lagged") {
      // after `lagged` the core's events for this were missed: read again what is shown.
      for (const sessionId of Object.keys(store.get().tabs)) {
        invalidate(sessionId);
        void poll(sessionId);
        readViewer(sessionId);
      }
    }
  });

  /** Keeps the report of the last drop on `sessionId`'s composer (the `ui/files` route reads it). */
  const noteDrop = (sessionId: string, report: DropReport): void => {
    store.set((s) => ({ ...s, drops: { ...s.drops, [sessionId]: report } }));
  };

  return {
    store,
    edit,
    want,
    image,
    noteDrop,
    invalidate,
    poll,
    layout,
    stop,
    actions: actions("ui"),
    debugApi: actions("debug-api"),
  } as const;
}

/** The files controller. */
export type Files = ReturnType<typeof createFiles>;
