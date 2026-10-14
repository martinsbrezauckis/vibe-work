/**
 * The outer git loop of the Changes tab: the UI side of the core's `git` command. The branch bar's
 * status per session, the Commit / Pull request / Bring into main dialogs, Push (the core asks the native
 * confirmation that names remote and branch), the PR card and its prepared messages.
 * Depends on: coreStore (`CoreController`), createStore, protocol types. createApp hands in `insertChip`:
 * a prepared message goes into the composer as a context chip, never sent or run as a command.
 * Called by: createApp (wiring), the views in `features/git/`, the `ui/git` route.
 * Invariants:
 * - Nothing leaves the machine from here: Push and Create PR call the core, which asks the native dialog (the
 *   Debug API acts as the user). A declined dialog shows no toast.
 * - The PR card asks the forge only on a press, and every 60 s while the session is shown and a card exists
 *   (the core keeps its own floor between asks).
 * - A commit names the files and the HEAD the dialog showed; `stale` re-reads the dialog and says so.
 */
import type {
  BringPlanView,
  CommandResult,
  CommitPlanView,
  GitRequest,
  GitResult,
  GitStatusView,
  PrPlanView,
  PrStatusView,
  PreparedKind,
} from "../protocol/core";
import { branchOf, httpsLink } from "../features/git/model";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import { pruneSessions } from "./prune";

/** The Commit dialog. */
export interface CommitDialog {
  kind: "commit";
  sessionId: string;
  repo: string;
  plan: CommitPlanView;
  message: string;
  /** Where the message came from (`model`, `fallback`, `user`), and a note. */
  messageFrom: "model" | "fallback" | "user" | "loading";
  messageNote: string | null;
  /** Paths left out (unticked). */
  excluded: readonly string[];
  runHooks: boolean;
}

/** The pull request dialog. */
export interface PrDialog {
  kind: "pr";
  sessionId: string;
  repo: string;
  plan: PrPlanView;
  title: string;
  body: string;
  base: string;
  draft: boolean;
}

/** The Bring into main dialog. */
export interface BringDialog {
  kind: "bring";
  sessionId: string;
  repo: string;
  plan: BringPlanView;
}

/** One open dialog. */
export type GitDialog = CommitDialog | PrDialog | BringDialog;

/** The git UI state. */
export interface GitUi {
  status: Readonly<Record<string, GitStatusView>>;
  selected: Readonly<Record<string, string>>;
  cards: Readonly<Record<string, PrStatusView>>;
  dialog: GitDialog | null;
  /** A request is running (per session). */
  busy: Readonly<Record<string, boolean>>;
  /** The last result or error line per session (shown in the bar). */
  notice: Readonly<Record<string, { kind: "ok" | "error"; text: string } | null>>;
  /** A failed status refresh keeps the last bar visible but blocks its actions. */
  loadError: Readonly<Record<string, string | null>>;
  /** What the last commit printed (hook output). */
  output: Readonly<Record<string, string>>;
}

const initial: GitUi = {
  status: {},
  selected: {},
  cards: {},
  dialog: null,
  busy: {},
  notice: {},
  loadError: {},
  output: {},
};

function put<T>(map: Readonly<Record<string, T>>, key: string, value: T): Record<string, T> {
  return { ...map, [key]: value };
}

/** How often a shown session's PR card is refreshed (ms). */
export const CARD_POLL_MS = 60_000;

/** Creates the git controller. `insertChip` puts a prepared message into a session's composer. */
export function createGit(deps: {
  core: CoreController;
  insertChip: (sessionId: string, label: string, text: string, origin: "ui" | "debug-api") => void;
}) {
  const { core, insertChip } = deps;
  const store = createStore<GitUi>(initial);

  const currentRepo = (id: string) =>
    store.get().selected[id] ?? store.get().status[id]?.repo ?? undefined;
  const isCurrent = (id: string, repo: string) => currentRepo(id) === repo;
  async function run(
    request: GitRequest,
    repo = currentRepo(request.sessionId),
  ): Promise<GitResult> {
    const result: CommandResult = await core.run({
      type: "git",
      request: repo === undefined ? request : { ...request, repo },
    });
    if (result.type !== "git") throw new Error(`unexpected answer ${result.type}`);
    return result.result;
  }
  const setBusy = (id: string, busy: boolean) => {
    store.set((s) => ({ ...s, busy: put(s.busy, id, busy) }));
  };
  const note = (id: string, kind: "ok" | "error", text: string) => {
    store.set((s) => ({ ...s, notice: put(s.notice, id, { kind, text }) }));
  };
  const actVersion: Record<string, number> = {};
  /** Runs `work` with the busy flag; errors become the bar's notice (a declined dialog says nothing). */
  async function act(id: string, work: () => Promise<void>): Promise<boolean> {
    const repo = currentRepo(id);
    const version = (actVersion[id] ?? 0) + 1;
    actVersion[id] = version;
    setBusy(id, true);
    try {
      await work();
      return true;
    } catch (thrown) {
      const error = toCallError(thrown).error;
      if (actVersion[id] === version && repo === currentRepo(id)) {
        if (error.code !== "confirmationDeclined") note(id, "error", error.message);
        if (error.code === "stale") await controller.load(id);
      }
      return false;
    } finally {
      if (actVersion[id] === version) setBusy(id, false);
    }
  }

  core.onSessionsGone((ids) => {
    store.set((s) => {
      const pruned = pruneSessions(
        s,
        ["status", "selected", "cards", "busy", "notice", "loadError", "output"],
        ids,
      );
      return pruned.dialog !== null && ids.includes(pruned.dialog.sessionId)
        ? { ...pruned, dialog: null }
        : pruned;
    });
  });
  const pending: Record<string, ReturnType<typeof setTimeout>> = {};
  const loadVersion: Record<string, number> = {};
  const pendingSelection: Record<string, string | undefined> = {};
  const selectionPromise: Record<string, Promise<void> | undefined> = {};
  const stopListening = core.listen((event) => {
    // The bar follows the folder: after any change event, read again (debounced).
    if (event.type === "changesUpdated" && store.get().status[event.sessionId] !== undefined) {
      const id = event.sessionId;
      clearTimeout(pending[id]);
      pending[id] = setTimeout(() => void controller.load(id), 400);
    }
  });

  const controller = {
    store,
    run,
    /** The repository stays selected until the user changes it or the session goes away. */
    async selectRepo(sessionId: string, repo: string): Promise<void> {
      if (pendingSelection[sessionId] === repo) return selectionPromise[sessionId];
      if (currentRepo(sessionId) === repo) {
        if (pendingSelection[sessionId] !== undefined) {
          loadVersion[sessionId] = (loadVersion[sessionId] ?? 0) + 1;
          pendingSelection[sessionId] = undefined;
          setBusy(sessionId, false);
        }
        return;
      }
      const version = (loadVersion[sessionId] ?? 0) + 1;
      loadVersion[sessionId] = version;
      actVersion[sessionId] = (actVersion[sessionId] ?? 0) + 1;
      pendingSelection[sessionId] = repo;
      store.set((s) => ({
        ...s,
        busy: put(s.busy, sessionId, true),
        notice: put(s.notice, sessionId, null),
        dialog: s.dialog?.sessionId === sessionId ? null : s.dialog,
      }));
      const task = (async () => {
        try {
          const result = await run({ op: "status", sessionId }, repo);
          if (result.kind !== "status" || result.status.repo !== repo)
            throw new Error("Repository status unavailable.");
          if (loadVersion[sessionId] !== version) return;
          store.set((s) => ({
            ...s,
            selected: put(s.selected, sessionId, repo),
            status: put(s.status, sessionId, result.status),
            cards: Object.fromEntries(Object.entries(s.cards).filter(([id]) => id !== sessionId)),
            loadError: put(s.loadError, sessionId, null),
          }));
        } catch (thrown) {
          if (loadVersion[sessionId] === version)
            note(sessionId, "error", toCallError(thrown).error.message);
        } finally {
          if (loadVersion[sessionId] === version) {
            pendingSelection[sessionId] = undefined;
            setBusy(sessionId, false);
          }
        }
      })();
      selectionPromise[sessionId] = task;
      await task;
      if (selectionPromise[sessionId] === task) selectionPromise[sessionId] = undefined;
    },

    /** Reads the branch bar of `sessionId`. */
    async load(sessionId: string): Promise<void> {
      if (pendingSelection[sessionId] !== undefined) return;
      const repo = currentRepo(sessionId);
      const version = (loadVersion[sessionId] ?? 0) + 1;
      loadVersion[sessionId] = version;
      try {
        const result = await run({ op: "status", sessionId }, repo);
        if (result.kind === "status")
          store.set((s) =>
            loadVersion[sessionId] === version &&
            (repo === undefined || s.selected[sessionId] === repo)
              ? {
                  ...s,
                  status: put(s.status, sessionId, result.status),
                  cards:
                    branchOf(s.status[sessionId] ?? result.status) !== branchOf(result.status) ||
                    (s.cards[sessionId]?.card?.head !== undefined &&
                      s.cards[sessionId].card.head !== branchOf(result.status))
                      ? Object.fromEntries(
                          Object.entries(s.cards).filter(([id]) => id !== sessionId),
                        )
                      : s.cards,
                  loadError: put(s.loadError, sessionId, null),
                  notice: s.loadError[sessionId] ? put(s.notice, sessionId, null) : s.notice,
                  selected:
                    result.status.repo === null
                      ? s.selected
                      : put(s.selected, sessionId, result.status.repo),
                }
              : s,
          );
      } catch (thrown) {
        if (loadVersion[sessionId] === version && store.get().status[sessionId] !== undefined) {
          const message = toCallError(thrown).error.message;
          store.set((s) => ({
            ...s,
            loadError: put(s.loadError, sessionId, message),
            notice: put(s.notice, sessionId, { kind: "error", text: message }),
          }));
        }
      }
    },

    /** Opens the Commit dialog and asks for a drafted message. */
    async openCommit(sessionId: string): Promise<void> {
      const repo = currentRepo(sessionId);
      if (repo === undefined) return;
      await act(sessionId, async () => {
        const result = await run({ op: "prepareCommit", sessionId }, repo);
        if (result.kind !== "commitPlan" || !isCurrent(sessionId, repo)) return;
        const plan = result.plan;
        store.set((s) => ({
          ...s,
          dialog: {
            kind: "commit",
            sessionId,
            repo,
            plan,
            message: "",
            messageFrom: "loading",
            messageNote: null,
            excluded: [],
            runHooks: plan.runHooks,
          },
        }));
        await controller.draft();
      });
    },

    /** Asks the core for a message for the ticked files (not when the user typed one). */
    async draft(): Promise<void> {
      const d = store.get().dialog;
      if (d?.kind !== "commit") return;
      const paths = d.plan.files.map((f) => f.path).filter((p) => !d.excluded.includes(p));
      if (paths.length === 0) return;
      try {
        const result = await run({ op: "draftMessage", sessionId: d.sessionId, paths }, d.repo);
        if (result.kind !== "draft" || !isCurrent(d.sessionId, d.repo)) return;
        store.set((s) =>
          s.dialog?.kind === "commit" &&
          s.dialog.sessionId === d.sessionId &&
          s.dialog.repo === d.repo &&
          s.dialog.messageFrom !== "user"
            ? {
                ...s,
                dialog: {
                  ...s.dialog,
                  message: result.message,
                  messageFrom: result.source,
                  messageNote: result.note,
                },
              }
            : s,
        );
      } catch (thrown) {
        if (isCurrent(d.sessionId, d.repo))
          note(d.sessionId, "error", toCallError(thrown).error.message);
      }
    },

    /** Edits the open Commit dialog. */
    editCommit(change: Partial<Pick<CommitDialog, "message" | "excluded" | "runHooks">>): void {
      store.set((s) => {
        if (s.dialog?.kind !== "commit") return s;
        const typed = change.message !== undefined ? { messageFrom: "user" as const } : {};
        return { ...s, dialog: { ...s.dialog, ...change, ...typed } };
      });
    },

    /** Commits as the dialog says; `then` continues with Push or the PR dialog. Returns true on success. */
    async commit(then: "none" | "push" | "pr" = "none"): Promise<boolean> {
      const d = store.get().dialog;
      if (d?.kind !== "commit") return false;
      const paths = d.plan.files.map((f) => f.path).filter((p) => !d.excluded.includes(p));
      const done = await act(d.sessionId, async () => {
        const result = await run(
          {
            op: "commit",
            sessionId: d.sessionId,
            paths,
            message: d.message,
            branch: d.plan.branch,
            expectedHead: d.plan.head,
            runHooks: d.runHooks,
          },
          d.repo,
        );
        if (result.kind !== "committed") return;
        if (!isCurrent(d.sessionId, d.repo)) return;
        store.set((s) => ({
          ...s,
          dialog: null,
          output: put(s.output, d.sessionId, result.output),
        }));
        // The bar's counts first, then the result line: they never disagree.
        await controller.load(d.sessionId);
        note(
          d.sessionId,
          "ok",
          `Committed ${String(paths.length)} ${paths.length === 1 ? "file" : "files"} (${result.commit.slice(0, 7)}).`,
        );
      });
      if (!done) {
        // A refused commit keeps the dialog open; a stale one is read again.
        const error = store.get().notice[d.sessionId];
        if (error?.kind === "error" && store.get().dialog?.kind === "commit")
          void controller.reopenCommit(d.sessionId);
        return false;
      }
      if (!isCurrent(d.sessionId, d.repo)) return false;
      if (then === "push") await controller.push(d.sessionId);
      if (then === "pr") await controller.openPr(d.sessionId);
      return true;
    },

    /** Re-reads the Commit dialog's file list, keeping the typed message. */
    async reopenCommit(sessionId: string): Promise<void> {
      try {
        const repo = store.get().dialog?.repo;
        if (repo === undefined) return;
        const result = await run({ op: "prepareCommit", sessionId }, repo);
        if (result.kind !== "commitPlan") return;
        store.set((s) =>
          s.dialog?.kind === "commit" && s.dialog.sessionId === sessionId && s.dialog.repo === repo
            ? { ...s, dialog: { ...s.dialog, plan: result.plan } }
            : s,
        );
      } catch {
        // The notice already says why.
      }
    },

    /** Pushes the branch (the core shows the native confirmation naming remote and branch). */
    async push(sessionId: string, remote?: string): Promise<boolean> {
      const repo = currentRepo(sessionId);
      if (repo === undefined) return false;
      return act(sessionId, async () => {
        const planned = await run(
          remote === undefined
            ? { op: "pushPlan", sessionId }
            : { op: "pushPlan", sessionId, remote },
          repo,
        );
        if (planned.kind !== "pushPlan" || !isCurrent(sessionId, repo)) return;
        const { plan } = planned.plan;
        const result = await run(
          {
            op: "push",
            sessionId,
            remote: plan.remote,
            commit: plan.commit,
          },
          repo,
        );
        if (!isCurrent(sessionId, repo)) return;
        if (result.kind === "pushed") note(sessionId, "ok", result.message);
        await controller.load(sessionId);
      });
    },

    /** Opens the pull request dialog. */
    async openPr(sessionId: string): Promise<void> {
      const repo = currentRepo(sessionId);
      if (repo === undefined) return;
      await act(sessionId, async () => {
        const result = await run({ op: "prPlan", sessionId }, repo);
        if (result.kind !== "prPlan" || !isCurrent(sessionId, repo)) return;
        const plan = result.plan;
        store.set((s) => ({
          ...s,
          dialog: {
            kind: "pr",
            sessionId,
            repo,
            plan,
            title: plan.title,
            body: plan.body,
            base: plan.base,
            draft: false,
          },
        }));
      });
    },

    /** Edits the open PR dialog. */
    editPr(change: Partial<Pick<PrDialog, "title" | "body" | "base" | "draft">>): void {
      store.set((s) =>
        s.dialog?.kind === "pr" ? { ...s, dialog: { ...s.dialog, ...change } } : s,
      );
    },

    /** Creates the pull request (gh) or opens the forge's page; returns the address shown or opened. */
    async createPr(): Promise<string | null> {
      const d = store.get().dialog;
      if (d?.kind !== "pr") return null;
      let address: string | null = null;
      await act(d.sessionId, async () => {
        const commit = d.plan.push?.commit ?? store.get().status[d.sessionId]?.state?.commit ?? "";
        const result = await run(
          {
            op: "createPr",
            sessionId: d.sessionId,
            remote: d.plan.remote,
            base: d.base,
            title: d.title,
            body: d.body,
            draft: d.draft,
            commit,
          },
          d.repo,
        );
        if (result.kind !== "prCreated") return;
        if (!isCurrent(d.sessionId, d.repo)) return;
        const { created } = result;
        address = created.url;
        store.set((s) => ({ ...s, dialog: null }));
        note(d.sessionId, "ok", created.message);
        // The forge page is our own address (vw-git `forge`), https only; checked again before it opens.
        const page = created.via === "browser" ? httpsLink(created.url) : null;
        if (page !== null) await core.client.openLink(page.url).catch(() => undefined);
        await controller.card(d.sessionId, created.via === "gh");
        await controller.load(d.sessionId);
      });
      return address;
    },

    /** Reads the PR card (`refresh`: ask the forge now). */
    async card(sessionId: string, refresh: boolean): Promise<void> {
      const repo = currentRepo(sessionId);
      const status = store.get().status[sessionId];
      const branch = status === undefined ? null : branchOf(status);
      if (repo === undefined || branch === null) return;
      try {
        const result = await run({ op: "prStatus", sessionId, refresh }, repo);
        if (result.kind === "prStatus")
          store.set((s) =>
            isCurrent(sessionId, repo) &&
            (s.status[sessionId] === undefined ? null : branchOf(s.status[sessionId])) === branch &&
            (result.status.card === null || result.status.card.head === branch)
              ? { ...s, cards: put(s.cards, sessionId, result.status) }
              : s,
          );
      } catch (thrown) {
        if (refresh && isCurrent(sessionId, repo))
          note(sessionId, "error", toCallError(thrown).error.message);
      }
    },

    /** Puts a prepared message into the composer (never sent). Returns the text, or null. */
    async prepare(
      sessionId: string,
      what: PreparedKind,
      origin: "ui" | "debug-api" = "ui",
    ): Promise<string | null> {
      let text: string | null = null;
      const repo = currentRepo(sessionId);
      if (repo === undefined) return null;
      await act(sessionId, async () => {
        const result = await run({ op: "prepareMessage", sessionId, what }, repo);
        if (result.kind !== "prepared" || !isCurrent(sessionId, repo)) return;
        text = result.text;
        const label =
          what === "checks"
            ? "Failing checks"
            : what === "review"
              ? "Review comments"
              : "Update the copy";
        insertChip(sessionId, label, result.text, origin);
        note(sessionId, "ok", "Added to the message box. Send it when you are ready.");
      });
      return text;
    },

    /** Opens the Bring into main dialog (the dry run). */
    async openBring(sessionId: string): Promise<void> {
      const repo = currentRepo(sessionId);
      if (repo === undefined || repo !== ".") return;
      await act(sessionId, async () => {
        const result = await run({ op: "bringPlan", sessionId }, repo);
        if (result.kind === "bringPlan")
          store.set((s) =>
            isCurrent(sessionId, repo)
              ? { ...s, dialog: { kind: "bring", sessionId, repo, plan: result.plan } }
              : s,
          );
      });
    },

    /** Brings the copy in as the dialog showed (the core asks the native confirmation). */
    async bringIn(): Promise<boolean> {
      const d = store.get().dialog;
      if (d?.kind !== "bring") return false;
      return act(d.sessionId, async () => {
        const result = await run(
          {
            op: "bringIn",
            sessionId: d.sessionId,
            baseCommit: d.plan.plan.baseCommit,
            branchCommit: d.plan.plan.branchCommit,
          },
          d.repo,
        );
        if (result.kind !== "brought") return;
        if (!isCurrent(d.sessionId, d.repo)) return;
        store.set((s) => ({ ...s, dialog: null }));
        note(
          d.sessionId,
          "ok",
          `${d.plan.plan.base} now has ${d.plan.plan.branch} (${result.commit.slice(0, 7)}).`,
        );
        await controller.load(d.sessionId);
      });
    },

    /** Saves a per-project setting from the Commit dialog. */
    async saveSetting(
      sessionId: string,
      change: { instruction?: string; credit?: boolean; runHooks?: boolean },
    ): Promise<void> {
      const dialog = store.get().dialog;
      const repo = dialog?.sessionId === sessionId ? dialog.repo : currentRepo(sessionId);
      if (repo === undefined) return;
      await act(sessionId, async () => {
        await run({ op: "setSettings", sessionId, ...change }, repo);
      });
    },

    /** Closes the open dialog. */
    close(): void {
      store.set((s) => ({ ...s, dialog: null }));
    },

    /** Stops following events. */
    stop(): void {
      stopListening();
      for (const timer of Object.values(pending)) clearTimeout(timer);
    },
  };
  return controller;
}

/** The controller type. */
export type Git = ReturnType<typeof createGit>;
