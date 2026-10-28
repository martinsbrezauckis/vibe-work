/**
 * The New chat screen as a working place before the first message: the core's owner id for its terminals and
 * browser (kept in the shell state as `newChatOwner`), the folder the core opens its terminals in, what is open
 * there (counts the Terminal and Browser features report), the hand-over at the first message and Close all.
 * Depends on: coreStore (`newChat` commands), shellStore, toastStore, files (the editor's open documents), the
 * session summaries. Called by: createApp (one per app), state/composer (Send), the New chat screen, the session
 * list's badge, terminal/preview register (hand-over hooks and counts), newChat.ts.
 * Invariants:
 * - Nothing here reaches Vibe: the owner id goes only to the core's terminal, preview and browser commands.
 * - The hand-over runs only after a session exists: the window's side moves in the same tick the start selects the
 *   new session (no frame without the panel), the core's side (`adopt`) before the first message is sent.
 * - Close all stops terminals only through its own command; leaving the screen, a failed send or a folder change
 *   stop nothing. Unsaved editor buffers are never dropped.
 */
import { toCallError } from "../protocol/client/types";
import type { NewChatView } from "../protocol/core";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import type { Files } from "./files";
import { summarize } from "./sessionSummary";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";
import { loadNewChat, saveNewChat, type SavedNewChat } from "./newChatRestore";

/** What is open on the New chat screen besides its files. */
export interface NewChatOpen {
  terminals: number;
  /** Browser pages with an address (the App preview and web tabs). */
  pages: number;
}

/** The controller's own state. */
export interface NewChatDraftState {
  open: NewChatOpen;
  /** The home folder new terminals open in while no folder is chosen. */
  home: string | null;
  /** Why the core did not take the folder (terminals then ask to choose it again), else `null`. */
  problem: string | null;
  /** Files open in the editor for the screen's folder. */
  files: number;
}

/** The state to read where no controller exists (no core): nothing open, no home folder. */
export const NO_NEW_CHAT = createStore<NewChatDraftState>({
  open: { terminals: 0, pages: 0 },
  home: null,
  problem: null,
  files: 0,
});

/** The editor key of the New chat screen's files in `folder`. */
function filesKeyOf(folder: string | null): string | null {
  return folder === null ? null : `files-workspace:${folder}`;
}

/** Creates the controller. */
export function createNewChatDraft(deps: {
  core: CoreController;
  shell: Shell;
  toasts: Toasts;
  files: Files | null;
  /** Keep what is open across a restart (`vw.newChatWorkspace`); off in tests. */
  persist: boolean;
  /** Offers the File tab (hidden until a file is open). */
  showFileTab: () => void;
}) {
  const { core, shell, toasts } = deps;
  const store = createStore<NewChatDraftState>({
    open: { terminals: 0, pages: 0 },
    home: null,
    problem: null,
    files: 0,
  });
  const hooks = new Set<(from: string, to: string) => void>();
  const view = (result: Awaited<ReturnType<CoreController["run"]>>): NewChatView => {
    if (result.type !== "newChat") throw new Error(`unexpected ${result.type} result`);
    return result.view;
  };
  const apply = (next: NewChatView): void => {
    shell.actions.setNewChatOwner(next.ownerId);
    // The home folder stays known after a folder is chosen (terminals opened there keep their tooltip).
    store.set((s) => ({ ...s, home: next.home ?? s.home }));
  };
  const tell = async (folder: string | null): Promise<void> => {
    try {
      const next = view(
        await core.run({
          type: "newChat",
          request: folder === null ? { op: "setFolder" } : { op: "setFolder", folder },
        }),
      );
      apply(next);
      // A declined restore clears the saved spelling; a newer folder choice keeps its own reply.
      if (folder !== null && next.folder == null && shell.store.get().newFolder === folder) {
        lastFolder = null;
        shell.actions.setNewFolder(null, "session");
      }
      store.set((s) => ({ ...s, problem: null }));
    } catch (thrown) {
      store.set((s) => ({ ...s, problem: toCallError(thrown).error.message }));
      // The owner id is still needed for the browser (it needs no folder).
      try {
        apply(view(await core.run({ type: "newChat", request: { op: "state" } })));
      } catch {
        // No core: the panels say so themselves.
      }
    }
  };

  // The start in progress: sessions that existed before it, so only the started one takes the panel.
  let expecting: ReadonlySet<string> | null = null;
  let hadOpen = false;
  const moveWindowSide = (to: string): void => {
    const s = shell.store.get();
    const from = s.newChatOwner;
    const filesKey = filesKeyOf(s.newFolder);
    const cwd = (() => {
      const entry = core.store.get().sessions[to];
      return entry === undefined ? null : summarize(entry).cwd;
    })();
    // Files open in the folder follow when the session works in that same folder (not in a new copy).
    const files = deps.files;
    if (files !== null && filesKey !== null && (cwd === null || cwd === s.newFolder))
      files.workspace.handOver(filesKey, to);
    if (from !== null) for (const hook of hooks) hook(from, to);
    shell.actions.handOverNewChat(to, filesKey);
  };
  let lastSelected = shell.store.get().selected;
  let lastFolder = shell.store.get().newFolder;
  const stopShell = shell.store.subscribe(() => {
    const s = shell.store.get();
    if (expecting !== null && lastSelected === null && s.selected !== null) {
      if (!expecting.has(s.selected)) {
        expecting = null;
        moveWindowSide(s.selected);
      }
    }
    lastSelected = s.selected;
    if (s.newFolder !== lastFolder) {
      lastFolder = s.newFolder;
      void tell(s.newFolder);
    }
  });

  const filesOpen = (): number => {
    const key = filesKeyOf(shell.store.get().newFolder);
    return key === null ? 0 : (deps.files?.workspace.store.get()[key]?.order.length ?? 0);
  };
  const countFiles = () => {
    const files = filesOpen();
    if (store.get().files !== files) store.set((s) => ({ ...s, files }));
  };
  // Web tab addresses, as the Browser reports them (kept across a restart).
  let addresses: readonly string[] = [];
  let restoring = false;
  const save = () => {
    if (!deps.persist || restoring) return;
    const s = shell.store.get();
    const panel = s.panels["new"];
    const key = filesKeyOf(s.newFolder);
    saveNewChat({
      folder: s.newFolder,
      tabs: panel?.open === true ? panel.tabs : [],
      active: panel?.open === true ? panel.active : null,
      files: key === null ? [] : (deps.files?.workspace.store.get()[key]?.order ?? []),
      pages: addresses,
    });
  };
  const changed = () => {
    countFiles();
    save();
  };
  const stopFiles = [
    shell.store.subscribe(changed),
    deps.files?.workspace.store.subscribe(changed),
  ];
  /** Reopens what the screen had open before the restart: folder, files, web tabs, then the panel's tabs. */
  const restore = async (saved: SavedNewChat): Promise<void> => {
    const folder = shell.store.get().newFolder;
    const key = filesKeyOf(folder);
    const files = deps.files;
    let shown = 0;
    if (files !== null && key !== null && folder !== null)
      for (const file of saved.files) {
        try {
          await files.actions.show(key, {
            file,
            display: file.startsWith(`${folder}/`) ? file.slice(folder.length + 1) : file,
            line: null,
          });
          shown += 1;
        } catch {
          // Moved or deleted since: not reopened.
        }
      }
    if (shown > 0) deps.showFileTab();
    const owner = shell.store.get().newChatOwner;
    if (owner !== null)
      for (const url of saved.pages) {
        try {
          await core.run({ type: "browser", request: { op: "openTab", sessionId: owner, url } });
        } catch {
          // Refused now (for example a local server that is gone): not reopened.
        }
      }
    for (const kind of saved.tabs) if (kind !== "file" || shown > 0) shell.actions.openTab(kind);
    if (saved.active !== null && (saved.active !== "file" || shown > 0))
      shell.actions.openTab(saved.active);
  };

  const controller = {
    store,
    /**
     * Asks the core for the owner id and gives it the folder; with persistence on, first reopens what the screen
     * had open before the restart (terminals come back as one fresh shell when the Terminal tab shows).
     */
    async start(): Promise<void> {
      const saved = deps.persist ? loadNewChat() : null;
      if (saved !== null && saved.folder !== null && shell.store.get().newFolder === null) {
        lastFolder = saved.folder;
        shell.actions.setNewFolder(saved.folder, "session");
      }
      await tell(shell.store.get().newFolder);
      if (saved === null || shell.store.get().selected !== null) return;
      restoring = true;
      try {
        await restore(saved);
      } finally {
        restoring = false;
        save();
      }
    },
    /** The Browser reports the New chat screen's web tab addresses (kept across a restart). */
    reportAddresses(urls: readonly string[]): void {
      if (urls.join("\n") === addresses.join("\n")) return;
      addresses = urls;
      save();
    },
    /** Called by a feature that keeps per-owner state: `from`'s becomes `to`'s at the first message. */
    onHandOver(hook: (from: string, to: string) => void): () => void {
      hooks.add(hook);
      return () => {
        hooks.delete(hook);
      };
    },
    /** A feature reports how many of its things are open on the New chat screen. */
    report(kind: keyof NewChatOpen, count: number): void {
      if (store.get().open[kind] !== count)
        store.set((s) => ({ ...s, open: { ...s.open, [kind]: count } }));
    },
    /** Whether a terminal or a page is open here (the screen then keeps its folder; the list shows a badge). */
    holds(): boolean {
      const { terminals, pages } = store.get().open;
      return terminals + pages > 0;
    },
    /** Open files of the screen's folder. */
    filesOpen,
    /** Send is about to start a session: the window's side moves when the start selects it. */
    expectStart(): void {
      expecting = new Set(Object.keys(core.store.get().sessions));
      // Taken now: once the window's side moves, the screen counts nothing open any more.
      hadOpen =
        controller.holds() || (shell.store.get().panels["new"]?.open ?? false) || filesOpen() > 0;
    },
    /**
     * The start ended: with `started`, the core moves terminals, browser and port lease to it (nothing stops) and
     * the screen gets a fresh owner; without, nothing moves.
     */
    async finishStart(started: string | null): Promise<void> {
      const pending = expecting;
      expecting = null;
      if (started === null) return;
      // The start did not select the session (an embedder, a test): move the window's side now.
      if (pending !== null) moveWindowSide(started);
      const had = hadOpen;
      try {
        apply(
          view(await core.run({ type: "newChat", request: { op: "adopt", sessionId: started } })),
        );
      } catch (thrown) {
        toasts.push({
          kind: "warn",
          bellKind: "failure",
          text: `The session started, but what was open here did not move to it: ${toCallError(thrown).error.message}`,
        });
        return;
      }
      store.set((s) => ({ ...s, open: { terminals: 0, pages: 0 } }));
      if (had) toasts.feedback("The session started. What you had open stays open.");
    },
    /**
     * Close all: the terminals stop, the pages close, the files close (unsaved ones stay open). The text and the
     * folder stay. Resolves to whether the core did it.
     */
    async closeAll(): Promise<boolean> {
      try {
        apply(view(await core.run({ type: "newChat", request: { op: "closeAll" } })));
      } catch (thrown) {
        toasts.push({
          kind: "warn",
          bellKind: "failure",
          text: `Not everything closed: ${toCallError(thrown).error.message}`,
        });
        return false;
      }
      const key = filesKeyOf(shell.store.get().newFolder);
      const files = deps.files;
      const kept = files !== null && key !== null ? files.workspace.closeClean(key) : 0;
      const panel = shell.store.get().panels["new"];
      if (panel !== undefined && shell.store.get().selected === null) {
        if (kept === 0) shell.actions.closePanel();
        else for (const kind of panel.tabs) if (kind !== "file") shell.actions.closeTab(kind);
      }
      store.set((s) => ({ ...s, open: { terminals: 0, pages: 0 } }));
      return true;
    },
    stop(): void {
      stopShell();
      for (const stop of stopFiles) stop?.();
    },
  };
  return controller;
}

/** The controller the app uses. */
export type NewChatDraft = ReturnType<typeof createNewChatDraft>;
