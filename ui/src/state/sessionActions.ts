/**
 * What the user can do to a session from the list: open, rename, pin, archive, delete (behind the
 * native confirmation), show its folder, start a session in a folder, trust a folder. Each action
 * sends typed commands through the core controller and folds the confirmed result into the model
 * (nothing is shown as done before Vibe says so).
 * Depends on: coreStore (`CoreController`), shellStore, toastStore, protocol types.
 * Called by: the list components, the palette, the UI-control `sessions` route.
 * Invariants:
 * - Delete never sends `session/delete` without a token from `requestConfirmation`, and the token
 *   exists only if the native dialog was approved; a declined dialog sends nothing and shows no toast.
 * - Opening a session marks it seen (`session/markAsSeen`) once, and only when Vibe reports it as
 *   unseen (`isUnseen`), so a read in the CLI is a read here too.
 * - The folder picker is native and owned by Rust; a path typed or sent by the webview never
 *   reaches the gate as "picked".
 * - Failures become one toast with the core's message; actions never throw to the caller.
 * - The core always knows which session the main view shows (`watchViewed`): that
 *   session's channel to Vibe stays open while it is shown.
 */
import type { Command, JsonValue, ProcessesResult } from "../protocol/core";
import { CoreCallError, toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { reportMissingChat } from "./sessionOpenError";
import { createStore } from "./createStore";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";

/** The actions, bound to one app's stores. */
export function createSessionActions(deps: {
  core: CoreController;
  shell: Shell;
  toasts: Toasts;
  /**
   * Right after a delete: the question about processes the session left running (they
   * stop before its copy may be removed). Its answer goes back to the `ui/sessions` route.
   */
  afterDelete?: ((id: string) => Promise<ProcessesResult | null>) | undefined;
}) {
  const { core, shell, toasts } = deps;
  const seenInFlight = new Set<string>();
  const trustGuidance = createStore<string | null>(null);
  // Ids whose last mark-as-seen failed: not retried by the watcher (no error loop), only by opening.
  const seenFailed = new Set<string>();

  const fail = (thrown: unknown, sessionId?: string): void => {
    const error = toCallError(thrown).error;
    if (error.code === "confirmationDeclined" || error.code === "accessModeUnverified") return;
    if (sessionId !== undefined && reportMissingChat(thrown, toasts, sessionId)) return;
    toasts.push({ kind: "error", bellKind: "failure", text: error.message });
  };
  const call = (method: string, params: Record<string, JsonValue>, token?: string) => {
    const command: Command = {
      type: "call",
      method,
      params,
      ...(token === undefined ? {} : { confirmationToken: token }),
    };
    return core.run(command);
  };
  const entryOf = (id: string) => core.store.get().sessions[id];
  /** Sessions with a "Try again" running (the row offers "Stop trying" meanwhile). */
  const retrying = createStore<ReadonlySet<string>>(new Set());
  const setRetrying = (id: string, on: boolean) => {
    retrying.set((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  };
  const number = (result: { type: string }, key: string): number | null => {
    const reply = result as { result?: Record<string, JsonValue> };
    const value = reply.result?.[key];
    return typeof value === "number" ? value : null;
  };

  const markSeen = async (id: string): Promise<void> => {
    if (core.store.get().held[id] !== undefined) return;
    const entry = entryOf(id);
    if (entry?.session["isUnseen"] !== true || seenInFlight.has(id) || seenFailed.has(id)) return;
    seenInFlight.add(id);
    try {
      await call("session/markAsSeen", { sessionId: id });
      core.patchSession(id, { isUnseen: false });
    } catch (thrown) {
      seenFailed.add(id);
      fail(thrown);
    } finally {
      seenInFlight.delete(id);
    }
  };

  const actions = {
    /** Children are removed only with their parent, including when called through a UI route. */
    deleteRefusal(id: string): string | null {
      const entry = entryOf(id);
      const parentId = entry?.session["parentSessionId"];
      const parent = typeof parentId === "string" ? entryOf(parentId) : undefined;
      const children = parent?.state?.["childSessions"];
      const owned =
        entry?.ownedByParent ??
        (parent !== undefined &&
          Array.isArray(children) &&
          children.some(
            (child) =>
              typeof child === "object" &&
              child !== null &&
              !Array.isArray(child) &&
              child["id"] === id,
          ));
      return owned ? "Subagent chats are removed when you delete their parent chat." : null;
    },
    markSeen,
    retrying,
    trustGuidance,

    /**
     * "Try again" for a stuck session: one bounded reopen attempt in the core. Returns true when
     * the session works again; a failure or a cancel leaves it stuck (a cancel shows no toast).
     */
    async retryReopen(id: string): Promise<boolean> {
      setRetrying(id, true);
      try {
        await core.run({ type: "retryReopen", sessionId: id });
        return true;
      } catch (thrown) {
        if (toCallError(thrown).error.code !== "cancelled") fail(thrown);
        return false;
      } finally {
        setRetrying(id, false);
      }
    },

    /** "Stop trying": ends the running "Try again" of a session at once. */
    async cancelReopen(id: string): Promise<boolean> {
      try {
        const result = await core.run({ type: "cancelReopen", sessionId: id });
        const reply = (result as { result?: Record<string, JsonValue> }).result;
        return reply?.["cancelled"] === true;
      } catch (thrown) {
        fail(thrown);
        return false;
      }
    },

    /** "Continue in a new session": same folder and profile, linked; opens the new session. */
    async continueInNew(id: string): Promise<string | null> {
      try {
        const result = await core.run({ type: "continueInNewSession", sessionId: id });
        if (result.type !== "session") return null;
        shell.actions.selectSession(result.snapshot.sessionId);
        return result.snapshot.sessionId;
      } catch (thrown) {
        fail(thrown);
        return null;
      }
    },

    /** Selects a session, loads its stored state if the app only has the listing row, marks it seen. */
    async open(id: string): Promise<void> {
      seenFailed.delete(id);
      shell.actions.selectSession(id);
      const entry = entryOf(id);
      if (entry === undefined) return;
      if (entry.state === null && !entry.localOnly) {
        try {
          await core.run({ type: "refreshSession", sessionId: id });
        } catch (thrown) {
          fail(thrown, id);
          return;
        }
      }
      await markSeen(id);
    },

    async rename(id: string, title: string): Promise<void> {
      const clean = title.trim();
      if (clean === "") return;
      try {
        await call("session/rename", { sessionId: id, title: clean });
        core.patchSession(id, { title: clean });
      } catch (thrown) {
        fail(thrown);
      }
    },

    async pin(id: string, pinned: boolean): Promise<void> {
      try {
        const result = await call("session/pin", { sessionId: id, pinned });
        core.patchSession(id, {
          pinnedAt: pinned ? (number(result, "pinnedAt") ?? Date.now()) : null,
        });
      } catch (thrown) {
        fail(thrown);
      }
    },

    async archive(id: string, archived: boolean): Promise<void> {
      try {
        const result = await call("session/archive", { sessionId: id, archived });
        core.patchSession(
          id,
          archived
            ? { archivedAt: number(result, "archivedAt") ?? Date.now(), pinnedAt: null }
            : { archivedAt: null },
        );
        if (archived && shell.store.get().selected === id) shell.actions.selectSession(null);
      } catch (thrown) {
        fail(thrown);
      }
    },

    /**
     * Delete: native confirmation first, then the call with its single-use token. Then the question about
     * processes the session left running, awaited so a Debug API request that deleted gets its answer.
     */
    async remove(id: string): Promise<boolean> {
      const reason = actions.deleteRefusal(id);
      if (reason !== null) return false;
      const params = { sessionId: id };
      try {
        const asked = await core.run({
          type: "requestConfirmation",
          method: "session/delete",
          params,
        });
        if (asked.type !== "confirmation") return false;
        await call("session/delete", params, asked.token);
      } catch (thrown) {
        fail(thrown);
        return false;
      }
      core.forget(id);
      if (shell.store.get().selected === id) shell.actions.selectSession(null);
      await deps.afterDelete?.(id);
      return true;
    },

    async revealFolder(id: string): Promise<void> {
      const cwd = entryOf(id)?.session["cwd"];
      if (typeof cwd !== "string") return;
      await actions.revealPath(cwd);
    },

    async revealPath(path: string): Promise<void> {
      try {
        await core.client.revealFolder(path);
      } catch (thrown) {
        fail(thrown);
      }
    },

    /** Native folder picker; remembers the choice as the folder of the next new session. */
    async pickFolder(): Promise<string | null> {
      try {
        const folder = await core.client.pickFolder();
        if (folder !== null) shell.actions.setNewFolder(folder, "picker");
        return folder;
      } catch (thrown) {
        fail(thrown);
        return null;
      }
    },

    /**
     * Starts a session in `cwd` (not trusted: trust is its own action) and opens it. A session in its own
     * copy of the repository starts through `worktrees.start`, never here. `origin` lowers the
     * start to the Debug API origin when a Debug API client chose the folder.
     */
    /** Starts a fresh chat with the current chat's project folder and access mode. */
    async startWithPlugins(id: string): Promise<string | null> {
      let entry = entryOf(id);
      if (entry?.runtime === undefined) {
        try {
          const reply = await core.run(
            { type: "call", method: "runtime/read", params: { sessionId: id } },
            { origin: "window" },
          );
          const result = reply.type === "reply" ? reply.result : null;
          const runtime =
            typeof result === "object" && result !== null && !Array.isArray(result)
              ? result["runtime"]
              : null;
          if (typeof runtime === "object" && runtime !== null && !Array.isArray(runtime))
            core.setRuntime(id, runtime);
          entry = entryOf(id);
        } catch (thrown) {
          fail(thrown);
          return null;
        }
      }
      const session = entry?.state?.["session"];
      const stateSession =
        typeof session === "object" && session !== null && !Array.isArray(session)
          ? session
          : entry?.session;
      const cwd = stateSession?.["cwd"];
      const workspace = stateSession?.["workspaceRoots"];
      const workspaceRoots = Array.isArray(workspace)
        ? workspace.filter((root): root is string => typeof root === "string")
        : [];
      const runtime = entry?.runtime;
      const active =
        typeof runtime === "object" && runtime !== null && !Array.isArray(runtime)
          ? runtime["activeAgent"]
          : null;
      const profile =
        typeof active === "object" && active !== null && !Array.isArray(active)
          ? active["name"]
          : null;
      if (typeof cwd !== "string" || typeof profile !== "string") {
        fail(
          new Error(
            "The chat's folder and access mode could not be read. Reopen it and try again.",
          ),
        );
        return null;
      }
      try {
        const result = await core.run(
          {
            type: "startSession",
            agentConfig: {
              cwd,
              agent: profile,
              ...(workspaceRoots.length === 0 ? {} : { workspaceRoots }),
            },
          },
          { origin: "window" },
        );
        if (result.type !== "session") return null;
        shell.actions.selectSession(result.snapshot.sessionId);
        return result.snapshot.sessionId;
      } catch (thrown) {
        fail(thrown);
        return null;
      }
    },
    async startIn(cwd: string): Promise<string | null> {
      try {
        const result = await core.run({ type: "startSession", agentConfig: { cwd } });
        if (result.type !== "session") return null;
        const started = result.snapshot.sessionId;
        // Every session has its own channel on the multiplexed server, so an earlier
        // empty session stays usable: nothing is dropped here.
        shell.actions.selectSession(started);
        return started;
      } catch (thrown) {
        fail(thrown);
        return null;
      }
    },

    /** What Vibe says about trust for `cwd` (status and the files that would load). */
    async trustStatus(cwd: string): Promise<{ status: string; files: string[] } | null> {
      try {
        const result = await call("workspace/trust/status", { cwd });
        const reply = (result as { result?: Record<string, JsonValue> }).result;
        const details = reply?.["details"];
        const files =
          typeof details === "object" && details !== null && !Array.isArray(details)
            ? details["detectedFiles"]
            : undefined;
        return {
          status: typeof reply?.["status"] === "string" ? reply["status"] : "untrusted",
          files: Array.isArray(files)
            ? files.filter((f): f is string => typeof f === "string")
            : [],
        };
      } catch (thrown) {
        fail(thrown);
        return null;
      }
    },

    /** The explicit trust action for one folder. Only a natively picked folder is accepted by the gate. */
    async trust(cwd: string): Promise<boolean> {
      trustGuidance.set(null);
      try {
        await call("workspace/trust/decision", { decision: "trust_cwd", cwd });
        return true;
      } catch (thrown) {
        if (thrown instanceof CoreCallError && thrown.error.code === "denied") {
          trustGuidance.set(
            "Only a folder you chose with Choose folder can be trusted. Choose it first.",
          );
        } else fail(thrown);
        return false;
      }
    },

    /**
     * Tells the core which session the main view shows whenever the selection changes, so the core
     * attaches and verifies its access mode immediately (`viewSession`), then keeps its channel open
     * however long it is idle. Failures use the usual toast. Returns a stop function.
     */
    watchViewed(): () => void {
      let last: string | null | undefined;
      const send = (): void => {
        const selected = shell.store.get().selected;
        if (selected === last) return;
        last = selected;
        const command: Command =
          selected === null
            ? { type: "viewSession" }
            : { type: "viewSession", sessionId: selected };
        core
          .run(command)
          .then((result) => {
            if (
              result.type === "session" &&
              result.snapshot.sessionId !== selected &&
              shell.store.get().selected === selected
            ) {
              shell.actions.selectSession(result.snapshot.sessionId);
            }
          })
          .catch((thrown: unknown) => {
            fail(thrown, selected ?? undefined);
          });
      };
      send();
      return shell.store.subscribe(send);
    },

    /**
     * Marks the open session seen whenever Vibe reports it unseen while the window has focus (a turn
     * that finishes in the session you are looking at must not leave a dot). Returns a stop function.
     */
    watchSeen(focused: () => boolean): () => void {
      const check = (): void => {
        const id = shell.store.get().selected;
        if (id !== null && focused()) void markSeen(id);
      };
      const offCore = core.store.subscribe(check);
      const offShell = shell.store.subscribe(check);
      return () => {
        offCore();
        offShell();
      };
    },
  };
  return actions;
}

/** The actions type. */
export type SessionActions = ReturnType<typeof createSessionActions>;
