/**
 * One sign-in at a time on the Connectors page. Two kinds:
 * - a Mistral connector: `signIn` -> Vibe sends the vendor's page address as the
 *   `connector_catalog/authUrl` notice -> the link dialog shows it in full -> the user signs in in the
 *   browser, where Mistral's own callback keeps the account link -> the page re-reads Mistral's catalog
 *   every 5 s (at most 5 minutes of key-ready time, or at "Check now") until the connector is ready;
 * - an MCP server added by address: `signInServer` -> the `mcp_catalog/authUrl` notice -> link dialog ->
 *   the browser hands the code to Vibe's own loopback port -> the core's `connectorSignIn` event ends it.
 * A sign-in Vibe asks for during a session's turn (a connector the gateway refused) arrives as an address
 * nobody here asked for: it is kept as a prompt the page shows, with a toast pointing there.
 * Depends on: coreStore (`listen`), createStore, transcriptStore (link dialog), toastStore, model/view.
 * Called by: connectors.ts.
 * Invariants: the address is only ever shown in the link dialog (never opened directly); the wait is
 * bounded while the key is usable; stopping never leaves a timer running.
 */
import type { ConnectorsRequest, ConnectorsResult, CoreEvent, JsonValue } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import { metaOf } from "../features/connectors/model/meta";
import { tileStatus } from "../features/connectors/model/view";
import type { ConnectorsState } from "./connectors";
import type { CoreController } from "./coreStore";
import type { Store } from "./createStore";
import type { Toasts } from "./toastStore";
import type { Transcript } from "./transcriptStore";

/** A sign-in on the page. */
export interface SignIn {
  kind: "connector" | "server";
  /** Alias (connector) or server name. */
  id: string;
  /** What the user knows it as. */
  name: string;
  sessionId: string | null;
  requestId: string | null;
  /** The sign-in page, once Vibe sent it. */
  url: string | null;
  phase: "starting" | "waiting" | "done" | "failed";
  /** Why it failed, in plain words. */
  message: string | null;
}

/** Timer functions (tests pass fakes). */
export interface Timers {
  setInterval(run: () => void, ms: number): unknown;
  clearInterval(handle: unknown): void;
}

/** How often and how long a Mistral connector sign-in is re-checked. */
export const POLL_MS = 5000;
export const POLL_MAX = 60;

const FAILED_CODES: Readonly<Record<string, string>> = {
  auth_url_unavailable:
    "Mistral did not give a sign-in page for this connector. Try again later, or manage it in Mistral's console.",
  stale_request: "The connector list changed while signing in. Try again.",
};

function obj(value: JsonValue): Readonly<Record<string, JsonValue | undefined>> {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : {};
}

function str(value: JsonValue | undefined): string | null {
  return typeof value === "string" ? value : null;
}

/** Creates the sign-in part of the connectors controller. */
export function createSignIns(deps: {
  core: CoreController;
  store: Store<ConnectorsState>;
  transcript: Transcript;
  toasts: Toasts;
  request: (origin: "ui" | "debug-api", req: ConnectorsRequest) => Promise<ConnectorsResult>;
  show: (result: ConnectorsResult) => void;
  /** Reads the view again with the session's live state (after a server sign-in ended). */
  reload: () => Promise<void>;
  /** Pauses hosted catalog polling while the model key is unavailable. */
  canUseKey?: () => boolean;
  timers?: Timers;
}) {
  const { core, store, transcript, toasts, request, show } = deps;
  const canUseKey = deps.canUseKey ?? (() => true);
  const timers: Timers = deps.timers ?? {
    setInterval: (run, ms) => window.setInterval(run, ms),
    clearInterval: (handle) => {
      window.clearInterval(handle as number);
    },
  };
  let poll: unknown = null;
  let polls = 0;
  const seenPrompts = new Map<string, string[]>();
  const promptNotices = new Map<string, number>();

  const clearPrompt = (alias: string): void => {
    const notice = promptNotices.get(alias);
    if (notice !== undefined) toasts.forget(notice);
    promptNotices.delete(alias);
    if (store.get().prompts.some((p) => p.id === alias))
      store.set((s) => ({ ...s, prompts: s.prompts.filter((p) => p.id !== alias) }));
  };

  // A catalog refresh can complete a prompted sign-in without another auth notice.
  store.subscribe(() => {
    const view = store.get().view;
    if (view === null) return;
    for (const prompt of store.get().prompts) {
      const connector = view.connectors.find((c) => c.alias === prompt.id);
      if (connector !== undefined && tileStatus(connector) === "connected") clearPrompt(prompt.id);
    }
  });

  const set = (signIn: SignIn | null): void => {
    store.set((s) => ({ ...s, signIn }));
  };
  const update = (p: Partial<SignIn>): void => {
    store.set((s) => (s.signIn === null ? s : { ...s, signIn: { ...s.signIn, ...p } }));
  };
  const stopPolling = (): void => {
    if (poll !== null) timers.clearInterval(poll);
    poll = null;
    polls = 0;
  };
  const finish = (phase: "done" | "failed", message: string | null): void => {
    stopPolling();
    update({ phase, message });
  };

  /** Re-reads Mistral's catalog; ends the wait when the connector is ready. */
  const check = async (origin: "ui" | "debug-api", sessionId: string | null): Promise<void> => {
    const current = store.get().signIn;
    if (current?.kind !== "connector" || !canUseKey()) return;
    try {
      const result = await request(origin, {
        op: "refresh",
        ...(sessionId === null ? {} : { sessionId }),
      });
      show(result);
      if (result.kind !== "view") return;
      const connector = result.view.connectors.find((c) => c.alias === current.id);
      if (connector !== undefined && tileStatus(connector) === "connected") {
        finish("done", `${current.name} is connected.`);
      }
    } catch {
      // A failed re-check is retried at the next tick; the bound ends the wait.
    }
  };

  core.listen((event: CoreEvent) => {
    if (event.type === "change" && event.change.kind === "removed") {
      for (const prompt of store.get().prompts) {
        if (prompt.sessionId === event.change.sessionId) clearPrompt(prompt.id);
      }
      return;
    }
    // after `lagged` the core's events for this were missed: read again what is shown.
    if (event.type === "lagged") {
      void deps.reload();
      return;
    }
    if (event.type === "connectorSignIn") {
      const current = store.get().signIn;
      if (current?.kind === "server" && current.id === event.name) {
        if (event.outcome === "signedIn") finish("done", `Signed in to ${current.name}.`);
        else if (event.outcome === "failed")
          finish("failed", event.message ?? "The sign-in failed.");
        void deps.reload();
      }
      return;
    }
    if (event.type !== "change" || event.change.kind !== "notice") return;
    const { method } = event.change;
    const params = obj(event.change.params);
    const current = store.get().signIn;
    if (method === "mcp_catalog/authUrl") {
      const name = str(params["name"]);
      const url = str(params["url"]);
      if (current?.kind === "server" && current.id === name && url !== null) {
        update({ url, phase: "waiting" });
        transcript.askLink(url);
      }
    } else if (
      method === "connector_catalog/authUrl" ||
      method === "connector_catalog/authFailed"
    ) {
      const alias = str(params["alias"]) ?? "";
      const requestId = str(params["requestId"]);
      const mine =
        current?.kind === "connector" &&
        current.id === alias &&
        (current.requestId === null || current.requestId === requestId);
      if (method === "connector_catalog/authFailed") {
        if (mine)
          finish(
            "failed",
            FAILED_CODES[str(params["code"]) ?? ""] ?? "The sign-in could not start.",
          );
        if (store.get().prompts.some((p) => p.id === alias && p.requestId === requestId))
          clearPrompt(alias);
        return;
      }
      const url = str(params["url"]);
      if (url === null) return;
      if (mine) {
        update({ url, phase: "waiting" });
        transcript.askLink(url);
        return;
      }
      const fingerprint = requestId ?? url;
      const seen = seenPrompts.get(alias) ?? [];
      if (seen.includes(fingerprint)) return;
      clearPrompt(alias);
      seenPrompts.set(alias, [...seen, fingerprint].slice(-32));
      // Asked by Vibe during a session's turn: keep it for the page, tell the user where it is.
      const name = metaOf(alias, alias).name;
      const prompt: SignIn = {
        kind: "connector",
        id: alias,
        name,
        sessionId: str(params["sessionId"]),
        requestId,
        url,
        phase: "waiting",
        message: null,
      };
      store.set((s) => ({ ...s, prompts: [...s.prompts.filter((p) => p.id !== alias), prompt] }));
      const notice = toasts.push({
        kind: "warn",
        bellKind: "approval",
        text: `${name} needs you to sign in (asked in a session). Open Tools to sign in.`,
      });
      promptNotices.set(alias, notice);
    }
  });

  return {
    /** Starts a Mistral connector's sign-in through `sessionId`. */
    async connector(
      origin: "ui" | "debug-api",
      alias: string,
      sessionId: string | null,
    ): Promise<void> {
      const name = metaOf(alias, alias).name;
      stopPolling();
      if (sessionId === null) {
        set({
          kind: "connector",
          id: alias,
          name,
          sessionId,
          requestId: null,
          url: null,
          phase: "failed",
          message: "Vibe signs connectors in through a session: open or start a session first.",
        });
        return;
      }
      set({
        kind: "connector",
        id: alias,
        name,
        sessionId,
        requestId: null,
        url: null,
        phase: "starting",
        message: null,
      });
      try {
        const result = await request(origin, { op: "signIn", sessionId, alias });
        if (result.kind === "signInStarted")
          update({ requestId: result.requestId, phase: "waiting" });
        poll = timers.setInterval(() => {
          if (!canUseKey()) return;
          polls += 1;
          if (polls > POLL_MAX) {
            finish("failed", "Not finished after 5 minutes. Click Sign in to try again.");
            return;
          }
          void check(origin, sessionId);
        }, POLL_MS);
      } catch (thrown) {
        const error = toCallError(thrown).error;
        if (error.code === "confirmationDeclined" || error.code === "keyService" || !canUseKey()) {
          stopPolling();
          set(null);
        } else finish("failed", error.message);
      }
    },
    /** Starts an added server's sign-in. */
    async server(origin: "ui" | "debug-api", serverName: string): Promise<void> {
      stopPolling();
      set({
        kind: "server",
        id: serverName,
        name: serverName,
        sessionId: null,
        requestId: null,
        url: null,
        phase: "starting",
        message: null,
      });
      try {
        await request(origin, { op: "signInServer", name: serverName });
        update({ phase: "waiting" });
      } catch (thrown) {
        const error = toCallError(thrown).error;
        if (error.code === "confirmationDeclined") set(null);
        else finish("failed", error.message);
      }
    },
    /** Shows the current sign-in page in the link dialog again. */
    openPage(): void {
      const url = store.get().signIn?.url ?? null;
      if (url !== null) transcript.askLink(url);
    },
    checkNow: (origin: "ui" | "debug-api", sessionId: string | null) => check(origin, sessionId),
    /** Ends the wait (a server sign-in also stops the core's wait; Vibe's own ends by its timeout). */
    async stop(origin: "ui" | "debug-api"): Promise<void> {
      const current = store.get().signIn;
      stopPolling();
      set(null);
      if (current?.kind === "connector") clearPrompt(current.id);
      if (current?.kind === "server" && current.phase !== "done") {
        await request(origin, { op: "cancelSignIn" }).then(show, () => undefined);
      }
    },
    /** Opens a sign-in Vibe asked for during a turn. */
    openPrompt(alias: string): void {
      const prompt = store.get().prompts.find((p) => p.id === alias);
      if (prompt?.url != null) transcript.askLink(prompt.url);
    },
  };
}
