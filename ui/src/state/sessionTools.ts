/**
 * Session tools in the window: "Export as Markdown" and temporary sessions, the UI side of the core's
 * `sessionTools` command and the app's `export_session` save dialog.
 * Depends on: coreStore (`CoreController`, its client), createStore, shellStore, toastStore, protocol types. Called by:
 * createApp, the session row menu, the command palette's actions, the new-chat view, the transcript banner, the
 * `ui/sessions` route.
 * Invariants:
 * - The window never names a destination: export goes through the native save dialog (Rust owns it); a Debug API
 *   client exports with `sessionTools {op: "export", destination}` through `/commands` instead.
 * - The temporary list is the core's (`sessionTools temporary`), read at start, after a temporary start and when
 *   sessions go away; a failed read keeps the last list.
 * - Failures become one toast; nothing throws to the caller.
 */
import type { SessionExportView, SessionToolsRequest, SessionToolsResult } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";

/** The UI state. */
export interface SessionToolsState {
  /** Temporary sessions (deleted when Vibe Work quits). */
  temporary: readonly string[];
  /** The export running now (session id), if any. */
  exporting: string | null;
  /** The new-chat view's "Temporary chat" switch (off again after each start). */
  newChatTemporary: boolean;
}

/** Stands in for the store when the app has no core (no temporary sessions, nothing exporting). */
export const NO_SESSION_TOOLS = createStore<SessionToolsState>({
  temporary: [],
  exporting: null,
  newChatTemporary: false,
});

/** "12 messages, 30 steps, 4 file changes" plus the secrets sentence, for the toast. */
export function exportSummary(view: SessionExportView): string {
  const counts = `${String(view.messages)} messages, ${String(view.steps)} steps, ${String(view.diffs)} file changes`;
  const secrets =
    view.secretsReplaced.length === 0 ? "" : " Known secret values were replaced with a marker.";
  const cut = view.cut ? " The oldest part was too long to include." : "";
  return `${counts}.${secrets}${cut}`;
}

/** Creates the controller. */
export function createSessionTools(deps: { core: CoreController; shell: Shell; toasts: Toasts }) {
  const { core, shell, toasts } = deps;
  const store = createStore<SessionToolsState>({
    temporary: [],
    exporting: null,
    newChatTemporary: false,
  });

  const run = async (
    request: SessionToolsRequest,
    origin?: "debug-api",
  ): Promise<SessionToolsResult> => {
    const result = await core.run(
      { type: "sessionTools", request },
      origin === undefined ? undefined : { origin },
    );
    if (result.type !== "sessionTools") throw new Error(`unexpected answer ${result.type}`);
    return result.result;
  };
  const fail = (thrown: unknown): void => {
    toasts.push({ kind: "error", bellKind: "failure", text: toCallError(thrown).error.message });
  };

  const refresh = async (): Promise<void> => {
    try {
      const result = await run({ op: "temporary" });
      if (result.kind === "temporary") store.set((s) => ({ ...s, temporary: result.sessionIds }));
    } catch {
      // Keeps the last list (module invariants).
    }
  };

  return {
    store,
    refresh,

    /** Sets the new-chat view's "Temporary chat" switch. */
    setNewChatTemporary(on: boolean): void {
      store.set((s) => ({ ...s, newChatTemporary: on }));
    },

    /** True when `id` is a temporary session. */
    isTemporary(id: string): boolean {
      return store.get().temporary.includes(id);
    },

    /**
     * Starts a temporary session in `cwd` and selects it; returns its id (`null` on failure, with a toast).
     * `origin` marks a start requested by a Debug API client.
     */
    async startTemporary(cwd: string, origin?: "debug-api"): Promise<string | null> {
      try {
        const result = await run({ op: "startTemporary", agentConfig: { cwd } }, origin);
        if (result.kind !== "started") return null;
        store.set((s) => ({
          ...s,
          temporary: [...s.temporary, result.sessionId],
          newChatTemporary: false,
        }));
        shell.actions.selectSession(result.sessionId);
        return result.sessionId;
      } catch (thrown) {
        fail(thrown);
        return null;
      }
    },

    /**
     * "Export as Markdown…": the native save dialog, then the file. Returns the answer (`null` when cancelled or
     * failed). The native dialog provides the saved path on success.
     */
    async exportSession(sessionId: string): Promise<SessionExportView | null> {
      if (store.get().exporting !== null) return null;
      store.set((s) => ({ ...s, exporting: sessionId }));
      try {
        const view = await core.client.exportSession(sessionId);
        return view;
      } catch (thrown) {
        fail(thrown);
        return null;
      } finally {
        store.set((s) => ({ ...s, exporting: null }));
      }
    },

    /** Reads the list now and again after a server restart or `lagged`; returns the function that stops it. */
    start(): () => void {
      void refresh();
      return core.listen((event) => {
        if (
          event.type === "lagged" ||
          (event.type === "server" && event.status.state === "running")
        )
          void refresh();
      });
    },

    /** Sessions that went away leave the list (they were deleted). */
    forget(ids: readonly string[]): void {
      if (ids.length === 0) return;
      store.set((s) => ({ ...s, temporary: s.temporary.filter((id) => !ids.includes(id)) }));
    },
  };
}

/** The controller type. */
export type SessionTools = ReturnType<typeof createSessionTools>;
