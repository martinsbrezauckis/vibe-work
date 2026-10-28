/**
 * Open documents and the focused editor pair. Wraps the single-file reader so every buffer keeps
 * its own baseline and requests; the existing file commands project the focused document.
 * Called by createFiles and editor tabs. Opening or hiding a document never discards its draft.
 */
import { watchFileActivity } from "./fileActivity";
import { toCallError } from "../protocol/client/types";
import { fitFileTabs } from "./fileTabVisibility";
import { createStore } from "./createStore";
import {
  createFileReader,
  moveFilesState,
  tabOf,
  RECENT,
  type FileReader,
  type Shown,
} from "./files";
import type { CoreController } from "./coreStore";

interface EditorPair {
  left: string;
  right: string;
  focus: "left" | "right";
  compare?: boolean;
}
export interface OpenDocument {
  shown: Shown;
  used: number;
  changed: boolean;
  saved?: string;
  /** The read-only side: the last commit (`saved`), the file on disk, or a commit from Files > History. */
  savedName?: "saved" | "disk" | "commit";
  /** The commit's short id, for `commit`. */
  savedLabel?: string;
  /** The rendered preview of `shown` (`preview:<file>`), beside its code. */
  preview?: true;
}
export interface FileWorkspace {
  order: readonly string[];
  documents: Readonly<Record<string, OpenDocument>>;
  active: string | null;
  split: EditorPair | null;
  ratio: number;
  visible: readonly string[];
  capacity: number;
  problem: string | null;
  closing: string | null;
  /** Why "Save and close" in the unsaved-changes dialog did not close (stale or failed save), else null. */
  closeProblem: string | null;
  folded: readonly ("left" | "right")[];
}
const EMPTY: FileWorkspace = {
  order: [],
  documents: {},
  active: null,
  split: null,
  visible: [],
  capacity: 5,
  problem: null,
  closing: null,
  closeProblem: null,
  folded: [],
  ratio: 0.5,
};

/** Independent readers share only the typed core connection and the isolated-view placement. */
export function createFileWorkspace(
  core: CoreController,
  base: FileReader,
  separate: () => boolean,
  openAs: () => "preview" | "code" | "side" = () => "preview",
) {
  const sessions = createStore<Readonly<Record<string, FileWorkspace>>>({});
  const store = createStore(base.store.get());
  const readers = new Map<string, FileReader>();
  const unsubscribers = new Map<string, () => void>();
  let clock = 0;
  const of = (id: string): FileWorkspace => sessions.get()[id] ?? EMPTY;
  const key = (id: string, file: string) => `${id}\u0000${file}`;
  const document = (id: string, file: string): FileReader | null =>
    readers.get(key(id, file)) ?? null;
  const current = (id: string): FileReader =>
    document(id, of(id).documents[of(id).active ?? ""]?.shown.file ?? "") ?? base;
  const publish = () => {
    const root = base.store.get();
    const tabs = { ...root.tabs };
    const edits = { ...root.edits };
    for (const [id, view] of Object.entries(sessions.get())) {
      const reader = current(id);
      const state = reader.store.get();
      const tab = tabOf(state, id);
      tabs[id] = {
        ...tab,
        recent: [...view.order]
          .sort((a, b) => (view.documents[b]?.used ?? 0) - (view.documents[a]?.used ?? 0))
          .slice(0, RECENT)
          .flatMap((file) => {
            const doc = view.documents[file];
            return doc ? [doc.shown] : [];
          }),
      };
      const draft = state.edits[id];
      if (draft === undefined) Reflect.deleteProperty(edits, id);
      else edits[id] = draft;
    }
    store.set({ ...root, tabs, edits });
  };
  const update = (id: string, change: (view: FileWorkspace) => FileWorkspace) => {
    sessions.set((all) => {
      const view = change(of(id));
      const visible = fitFileTabs(
        view.order,
        view.visible,
        view.capacity,
        view.active,
        (file) => dirty(id, file),
        (file) => view.documents[file]?.used ?? 0,
      );
      return { ...all, [id]: { ...view, visible } };
    });
    publish();
  };
  const stopBase = base.store.subscribe(publish);
  const activate = (id: string, file: string) => {
    const doc = of(id).documents[file];
    if (!doc) return;
    update(id, (view) => ({
      ...view,
      active: file,
      // An explanation (no saved version to compare) belongs to the file it was given for.
      problem: view.active === file ? view.problem : null,
      documents: { ...view.documents, [file]: { ...doc, used: ++clock } },
      split:
        view.split === null
          ? null
          : view.split.left === file || view.split.right === file
            ? { ...view.split, focus: view.split.left === file ? "left" : "right" }
            : view.split.compare
              ? null
              : { ...view.split, [view.split.focus]: file },
    }));
  };
  const ensure = (id: string, shown: Shown) => {
    let reader = document(id, shown.file);
    if (reader === null) {
      reader = createFileReader(core);
      readers.set(key(id, shown.file), reader);
      unsubscribers.set(key(id, shown.file), reader.store.subscribe(publish));
      update(id, (view) => ({
        ...view,
        order: [...view.order, shown.file],
        documents: { ...view.documents, [shown.file]: { shown, used: ++clock, changed: false } },
      }));
    }
    return reader;
  };
  const dirty = (id: string, file: string): boolean => {
    const draft = document(id, file)?.store.get().edits[id];
    return draft !== undefined && (draft.saving || draft.text !== draft.base.text);
  };
  const restore = (id: string) => {
    update(id, (v) => ({
      ...v,
      split: null,
      folded: [],
      active: v.split?.compare ? v.split.right : v.active,
    }));
  };
  const close = (id: string, file: string): boolean => {
    if (dirty(id, file)) return false;
    const view = of(id);
    // A file's preview goes with it.
    const gone = [file, `preview:${file}`];
    const order = view.order.filter((f) => !gone.includes(f));
    const docs = { ...view.documents };
    for (const g of gone) Reflect.deleteProperty(docs, g);
    const pair = view.split;
    const candidate = pair?.left === file ? pair.right : pair?.right === file ? pair.left : null;
    const other =
      candidate !== file && candidate !== null && order.includes(candidate) ? candidate : null;
    update(id, (v) => ({
      ...v,
      order,
      documents: docs,
      active:
        v.active !== null && gone.includes(v.active)
          ? (other ?? order[Math.max(0, view.order.indexOf(file) - 1)] ?? null)
          : v.active,
      split: pair?.left === file || pair?.right === file ? null : v.split,
    }));
    const reader = document(id, file);
    unsubscribers.get(key(id, file))?.();
    reader?.stop();
    readers.delete(key(id, file));
    unsubscribers.delete(key(id, file));
    publish();
    return true;
  };
  /** Code on the left, its rendered preview on the right (so it counts as the second editor). */
  const previewToSide = (id: string, shown: Shown) => {
    const reader = ensure(id, shown);
    reader.edit.begin(id);
    const preview = `preview:${shown.file}`;
    update(id, (v) => ({
      ...v,
      order: v.order.includes(preview) ? v.order : [...v.order, preview],
      documents: {
        ...v.documents,
        [preview]: { shown, preview: true, used: ++clock, changed: false },
      },
      split: { left: shown.file, right: preview, focus: "left" },
      active: shown.file,
      problem: null,
    }));
  };
  /** Opens (or reuses) the document for `shown`, focuses it and marks it most recently used. */
  const place = (id: string, shown: Shown): FileReader => {
    const reader = ensure(id, shown);
    activate(id, shown.file);
    update(id, (v) => ({
      ...v,
      documents: {
        ...v.documents,
        [shown.file]: {
          shown,
          used: ++clock,
          changed: v.documents[shown.file]?.changed ?? false,
        },
      },
    }));
    return reader;
  };
  const actions = (origin: "ui" | "debug-api"): FileReader["actions"] => {
    const forSession = (id: string) =>
      origin === "ui" ? current(id).actions : current(id).debugApi;
    const root = origin === "ui" ? base.actions : base.debugApi;
    return {
      ...root,
      async show(id, shown) {
        const reader = place(id, shown);
        await (origin === "ui" ? reader.actions : reader.debugApi).show(id, shown);
        const kind = tabOf(reader.store.get(), id).content?.kind;
        if (!separate()) return;
        // Code opens in the editor; HTML and Markdown as the setting says (a preview by default).
        const previewed = kind === "markdown" || kind === "html";
        if (kind === "text" || (previewed && openAs() === "code")) reader.edit.begin(id);
        else if (previewed && openAs() === "side") previewToSide(id, shown);
      },
      async showForEdit(id, shown) {
        // The document's own showForEdit keeps the single-reader contract: it begins the edit only for
        // complete editable text and otherwise leaves "The editor did not open. <reason>" on the tab.
        const reader = place(id, shown);
        await (origin === "ui" ? reader.actions : reader.debugApi).showForEdit(id, shown);
      },
      async openText(id, text, line = null, end) {
        const ref = await root.resolveRef(id, text);
        if (ref.status !== "ok") return false;
        await this.show(id, {
          file: ref.file,
          display: ref.display,
          line,
          ...(end === undefined ? {} : { end }),
        });
        return true;
      },
      async openAt(id, text, line) {
        if (!(await this.openText(id, text, line))) return false;
        await forSession(id).setMode(id, "source");
        return true;
      },
      note: (id, text) => {
        forSession(id).note(id, text);
      },
      reload: (id) => forSession(id).reload(id),
      reloadEdit: (id) => forSession(id).reloadEdit(id),
      setMode: (id, mode) => forSession(id).setMode(id, mode),
      external: (id, what, handle) => forSession(id).external(id, what, handle),
      write: (id, text, fingerprint) => {
        const reader = fingerprint ? document(id, fingerprint.file) : null;
        const target = reader
          ? origin === "ui"
            ? reader.actions
            : reader.debugApi
          : forSession(id);
        return target.write(id, text, fingerprint);
      },
      close(id) {
        const file = of(id).active;
        if (file !== null && !close(id, file))
          throw Error("Save or Cancel your edit before closing the file.");
      },
    };
  };
  const workspace = {
    store: sessions,
    /**
     * Moves every open document of `from` to `to`, unsaved edits included (the New chat screen's files become
     * the session's at its first message). Nothing is read again or closed.
     */
    handOver(from: string, to: string) {
      const view = sessions.get()[from];
      if (view === undefined || from === to) return;
      for (const file of view.order) {
        const reader = readers.get(key(from, file));
        const off = unsubscribers.get(key(from, file));
        readers.delete(key(from, file));
        unsubscribers.delete(key(from, file));
        if (reader === undefined) continue;
        reader.store.set((s) => moveFilesState(s, from, to));
        readers.set(key(to, file), reader);
        if (off !== undefined) unsubscribers.set(key(to, file), off);
      }
      sessions.set((all) => {
        const next = { ...all, [to]: view };
        Reflect.deleteProperty(next, from);
        return next;
      });
      publish();
    },
    of,
    activate,
    close,
    dirty,
    restore,
    /** Closes every document of `id` without unsaved edits; returns how many stay open (unsaved). */
    closeClean(id: string): number {
      return of(id).order.filter((file) => !close(id, file)).length;
    },
    fold(id: string, side: "left" | "right") {
      update(id, (v) => ({
        ...v,
        folded: v.folded.includes(side) ? v.folded.filter((s) => s !== side) : [...v.folded, side],
      }));
    },
    requestClose(id: string, file: string) {
      if (!close(id, file)) update(id, (v) => ({ ...v, closing: file, closeProblem: null }));
    },
    cancelClose(id: string) {
      update(id, (v) => ({ ...v, closing: null, closeProblem: null }));
    },
    /** Keeps the unsaved-changes dialog open and says why Save and close did not close the file. */
    closeFailed(id: string, problem: string) {
      update(id, (v) => ({ ...v, closeProblem: problem }));
    },
    async compareSaved(id: string) {
      const file = of(id).active;
      const doc = file === null ? undefined : of(id).documents[file];
      if (!doc || file === null) return;
      try {
        const result = await core.run({
          type: "files",
          request: { op: "readSaved", sessionId: id, file: doc.shown.file },
        });
        if (of(id).active !== file || !of(id).documents[file]) return;
        if (result.type !== "files" || result.result.kind !== "saved") return;
        if (result.result.text === null)
          update(id, (v) => ({
            ...v,
            problem: result.result.kind === "saved" ? result.result.note : null,
          }));
        else {
          document(id, doc.shown.file)?.edit.begin(id);
          this.compare(id, result.result.text);
        }
      } catch (error) {
        update(id, (v) => ({ ...v, problem: toCallError(error).error.message }));
      }
    },
    capacity(id: string, capacity: number) {
      if (of(id).capacity !== capacity) update(id, (v) => ({ ...v, capacity }));
    },
    focus(id: string, side: "left" | "right") {
      const pair = of(id).split;
      if (!pair) return;
      update(id, (v) => ({ ...v, active: pair[side], split: { ...pair, focus: side } }));
    },
    ratio(id: string, ratio: number) {
      update(id, (v) => ({ ...v, ratio: Math.max(0.25, Math.min(0.75, ratio)) }));
    },
    markChanged(id: string, file: string, changed = true) {
      const doc = of(id).documents[file];
      if (!doc) return;
      update(id, (v) => ({
        ...v,
        documents: { ...v.documents, [file]: { ...doc, changed } },
      }));
    },
    closeSaved(id: string) {
      for (const file of [...of(id).order]) if (!dirty(id, file)) close(id, file);
    },
    previewToSide,
    async openToSide(id: string, shown: Shown) {
      const previous = of(id);
      const left = previous.split?.left ?? previous.active;
      const reader = ensure(id, shown);
      await reader.actions.show(id, shown);
      reader.edit.begin(id);
      update(id, (v) => ({
        ...v,
        active: shown.file,
        problem: null,
        split: left === null ? null : { left, right: shown.file, focus: "right" },
      }));
    },
    compare(
      id: string,
      saved: string,
      savedName: "saved" | "disk" | "commit" = "saved",
      savedLabel?: string,
    ) {
      const active = of(id).active;
      const file = active === null ? null : of(id).documents[active]?.shown.file;
      if (!file) return;
      const shown = of(id).documents[file]?.shown;
      if (!shown) return;
      const savedId = `${savedName}:${file}`;
      update(id, (v) => ({
        ...v,
        order: v.order.includes(savedId) ? v.order : [...v.order, savedId],
        documents: {
          ...v.documents,
          [savedId]: {
            shown,
            saved,
            savedName,
            ...(savedLabel === undefined ? {} : { savedLabel }),
            used: ++clock,
            changed: false,
          },
        },
        split: { left: savedId, right: file, focus: "right", compare: true },
        active: file,
        problem: null,
      }));
    },
  };
  const stopActivity = watchFileActivity(
    core,
    () => sessions.get(),
    (id, file) => {
      workspace.markChanged(id, file);
    },
  );
  const stopPrune = core.onSessionsGone((ids) => {
    for (const id of ids) {
      for (const file of of(id).order) {
        const reader = document(id, file);
        reader?.stop();
        unsubscribers.get(key(id, file))?.();
        readers.delete(key(id, file));
        unsubscribers.delete(key(id, file));
      }
    }
    sessions.set((all) =>
      Object.fromEntries(Object.entries(all).filter(([id]) => !ids.includes(id))),
    );
    publish();
  });
  return {
    ...base,
    store,
    document,
    workspace,
    actions: actions("ui"),
    debugApi: actions("debug-api"),
    edit: {
      begin: (id: string) => current(id).edit.begin(id),
      update: (id: string, patch: Parameters<FileReader["edit"]["update"]>[1]) => {
        current(id).edit.update(id, patch);
      },
      cancel: (id: string) => {
        current(id).edit.cancel(id);
      },
      apply: (id: string, change: Parameters<FileReader["edit"]["apply"]>[1]) => {
        current(id).edit.apply(id, change);
      },
      takeDisk: (id: string) => current(id).edit.takeDisk(id),
    },
    poll: (id: string) => current(id).poll(id),
    stop() {
      stopActivity();
      stopBase();
      stopPrune();
      base.stop();
      for (const reader of readers.values()) reader.stop();
      for (const off of unsubscribers.values()) off();
    },
  };
}
