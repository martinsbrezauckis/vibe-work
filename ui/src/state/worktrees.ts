/**
 * Worktree per session: the UI side of the core's `worktrees` command. Starting a session in its
 * own copy (and sending its first message), the end-of-session choice after archive or delete ("Keep the
 * worktree" / "Remove it", with the inspection shown first), and the startup list of copies whose session
 * was deleted, including copies the user kept. The UI counts accumulated copies on startup and after core worktree
 * changes.
 * Depends on: coreStore (`CoreController`), shellStore, toastStore, createStore, protocol types.
 * Called by: createApp (wiring), sessionActions (`ended` after archive/delete), the worktree views.
 * Invariants:
 * - A start or read for a folder a Debug API client chose carries the Debug API origin.
 * - A removal names the inspection it was shown (its fingerprint); the core compares it and the native dialog
 *   names the files again. Nothing is removed from here without a click on "Remove".
 * - Failures become one toast; a declined native dialog shows none.
 */
import type {
  CommandResult,
  IncludeReport,
  JsonValue,
  OrphanWorktree,
  WorktreeInspection,
  WorktreesRequest,
  WorktreesResult,
} from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";
import { classify } from "../features/composer/model/message";
import { createWorktreeNotifications } from "./worktreeNotifications";

/** The open end-of-session choice. */
export interface EndChoice {
  sessionId: string;
  /** Why it is asked. */
  after: "archive" | "delete" | "startup";
  /** The inspection (null while it loads). */
  inspection: WorktreeInspection | null;
  /** A removal or keep is running. */
  busy: boolean;
}

/** The worktree UI state. */
export interface WorktreesState {
  /** The dialog, when open. */
  end: EndChoice | null;
  /** Leftover and kept copies with no live session. */
  orphans: readonly OrphanWorktree[];
  /** A fresh "New chat here" choice resets the copy switch, even for the same folder. */
  newChatRequest: number;
  /** Explicit choice from the Git tree, applied when its folder's composer opens. */
  newChatCopy: boolean | null;
  /** What `.worktreeinclude` copied into each session's copy this run (not kept across restarts). */
  included: Readonly<Record<string, IncludeReport>>;
}

/** Stable empty state for views that can render without a core. */
export const NO_WORKTREES = createStore<WorktreesState>({
  end: null,
  orphans: [],
  newChatRequest: 0,
  newChatCopy: null,
  included: {},
});

/** What starting a session in a copy needs. */
export interface CopyStart {
  cwd: string;
  base: string;
  branch: string;
  /** Sent as the first turn when not empty. */
  firstMessage: string;
}

/** Creates the worktree controller. */
export function createWorktrees(deps: {
  core: CoreController;
  shell: Shell;
  toasts: Toasts;
  persist?: boolean;
  /** App commands in the form's first-message field use the same handler as the composer. */
  runCommand?: (
    sessionId: string,
    name: string,
    args: string,
    origin?: "debug-api",
  ) => Promise<unknown>;
}) {
  const { core, shell, toasts } = deps;
  const store = createStore<WorktreesState>({
    end: null,
    orphans: [],
    newChatRequest: 0,
    newChatCopy: null,
    included: {},
  });
  const notices = createWorktreeNotifications(deps.persist ?? true);
  let counting = Promise.resolve();

  const fail = (thrown: unknown): void => {
    const error = toCallError(thrown).error;
    if (error.code === "confirmationDeclined") return;
    toasts.push({ kind: "error", bellKind: "failure", text: error.message });
  };

  /** One request; `origin` lowers it to the Debug API origin (a folder a Debug API client chose). */
  async function run(request: WorktreesRequest, origin?: "debug-api"): Promise<WorktreesResult> {
    const result: CommandResult = await core.run(
      { type: "worktrees", request },
      origin === undefined ? undefined : { origin },
    );
    if (result.type !== "worktrees") throw new Error(`unexpected answer ${result.type}`);
    return result.result;
  }

  async function inspect(choice: EndChoice): Promise<void> {
    try {
      const result = await run({ op: "inspect", sessionId: choice.sessionId });
      if (result.kind !== "inspection") return;
      store.set((s) =>
        s.end?.sessionId === choice.sessionId
          ? { ...s, end: { ...choice, inspection: result.inspection } }
          : s,
      );
    } catch (thrown) {
      store.set((s) => ({ ...s, end: null }));
      fail(thrown);
    }
  }

  /** Serialize reads so an older count cannot undo a newer notification threshold. */
  function countCopies(): Promise<void> {
    counting = counting.then(async () => {
      try {
        const result = await run({ op: "count" });
        if (result.kind === "count" && notices.observe(result.count) !== null) {
          toasts.push({
            kind: "info",
            bellKind: "worktrees",
            text: `${String(result.count)} worktrees`,
            action: "sessions",
          });
        }
      } catch {
        // Worktrees off or unavailable: keep the threshold until the next successful read.
      }
    });
    return counting;
  }

  const actions = {
    store,

    /** Runs a worktree request (views use it for options and suggestions). */
    run,

    /** Count at app start and after creation/removal, including Debug API commands; no polling. */
    watch(): () => void {
      void countCopies();
      return core.listen((event) => {
        if (
          event.type === "worktreesChanged" ||
          event.type === "lagged" ||
          (event.type === "server" && event.status.state === "running")
        )
          void countCopies();
      });
    },

    /** Chooses an existing copy for the next chat, without creating another copy. */
    newChatHere(
      path: string,
      copy: boolean | null = null,
      origin: "ui" | "debug-api" = "ui",
    ): void {
      shell.actions.setNewFolder(path, origin === "debug-api" ? "debug-api" : "session");
      store.set((s) => ({ ...s, newChatRequest: s.newChatRequest + 1, newChatCopy: copy }));
    },

    /**
     * Starts a session in its own copy, opens it and sends the first message (if any). Returns the new
     * session's id, `null` on failure (a toast says why). `origin: "debug-api"` when a Debug API client
     * chose the folder: the core then asks the native dialog. The first message was typed by
     * the user in the form, so it goes out with the window's origin.
     */
    async start(spec: CopyStart, origin?: "debug-api"): Promise<string | null> {
      try {
        const agentConfig: JsonValue = { cwd: spec.cwd };
        const result = await run(
          {
            op: "start",
            agentConfig,
            base: spec.base,
            branch: spec.branch,
          },
          origin,
        );
        if (result.kind !== "started") return null;
        const id = result.snapshot.sessionId;
        if (result.include.found) {
          store.set((s) => ({ ...s, included: { ...s.included, [id]: result.include } }));
        }
        shell.actions.selectSession(id);
        await actions.loadOrphans();
        const text = spec.firstMessage.trim();
        if (text !== "") {
          const intent = classify(text, 0, 0);
          if (intent.kind === "command") {
            if (deps.runCommand === undefined)
              throw new Error("Run this app command from the chat composer.");
            await deps.runCommand(id, intent.name, intent.args, origin);
          } else {
            await core.run({ type: "startTurn", sessionId: id, message: [{ type: "text", text }] });
          }
        }
        return id;
      } catch (thrown) {
        fail(thrown);
        return null;
      }
    },

    /**
     * After a session was archived or deleted: when it works in a copy, open the Keep / Remove choice
     * with the inspection. Sessions without a copy (or with a decision already made) ask nothing.
     */
    async ended(sessionId: string, after: "archive" | "delete"): Promise<void> {
      let result: WorktreesResult;
      try {
        result = await run({ op: "info", sessionId });
      } catch {
        // No answer about a copy (worktrees off, Vibe busy): nothing to ask now; a deleted session's copy
        // shows up in the startup list.
        return;
      }
      if (result.kind !== "info" || result.info === null) return;
      if (result.info.state !== "ready" || result.info.decision !== null) return;
      await actions.loadOrphans();
      const choice: EndChoice = { sessionId, after, inspection: null, busy: false };
      store.set((s) => ({ ...s, end: choice }));
      await inspect(choice);
    },

    /** Opens the existing inspection and Keep / Remove choice for any listed copy. */
    async review(orphan: OrphanWorktree): Promise<void> {
      const choice: EndChoice = {
        sessionId: orphan.sessionId,
        after: "startup",
        inspection: null,
        busy: false,
      };
      store.set((s) => ({ ...s, end: choice }));
      await inspect(choice);
    },

    /** "Keep the worktree": remembered, never asked again. */
    async keep(): Promise<void> {
      const end = store.get().end;
      if (end === null) return;
      store.set((s) => ({ ...s, end: s.end === null ? null : { ...s.end, busy: true } }));
      try {
        await run({ op: "keep", sessionId: end.sessionId });
        store.set((s) => ({
          ...s,
          end: null,
        }));
        await actions.loadOrphans();
      } catch (thrown) {
        store.set((s) => ({ ...s, end: s.end === null ? null : { ...s.end, busy: false } }));
        fail(thrown);
      }
    },

    /** "Remove it": names the inspection the user saw (its fingerprint); the native dialog names the files. */
    async remove(): Promise<void> {
      const end = store.get().end;
      const inspection = end?.inspection;
      if (end === null || inspection === null || inspection === undefined) return;
      store.set((s) => ({ ...s, end: s.end === null ? null : { ...s.end, busy: true } }));
      try {
        await run({
          op: "remove",
          sessionId: end.sessionId,
          fingerprint: inspection.fingerprint,
        });
        store.set((s) => ({
          ...s,
          end: null,
          orphans: s.orphans.filter((o) => o.sessionId !== end.sessionId),
        }));
      } catch (thrown) {
        store.set((s) => ({ ...s, end: s.end === null ? null : { ...s.end, busy: false } }));
        const error = toCallError(thrown).error;
        // The copy changed since it was shown: show the new state instead of guessing.
        if (error.code === "stale") void inspect({ ...end, busy: false });
        fail(thrown);
      }
    },

    /** Closes the dialog without deciding (asked again at the next start if the session is deleted). */
    dismiss(): void {
      store.set((s) => ({ ...s, end: null }));
    },

    /** Loads the startup list of orphan copies. */
    async loadOrphans(): Promise<void> {
      try {
        const result = await run({ op: "orphans" });
        if (result.kind === "orphans") store.set((s) => ({ ...s, orphans: result.orphans }));
      } catch {
        // Worktrees off (no app data) or Vibe not ready: no list, nothing to tell.
      }
    },
  };
  return actions;
}

/** The worktree controller. */
export type Worktrees = ReturnType<typeof createWorktrees>;
