/**
 * Dev-mode client: a normal browser talking to a real core through the Debug API.
 * The browser cannot call the Debug API directly (it refuses any request with an `Origin` header
 * and needs a bearer token), so the Vite dev server proxies `/__vw/*` to it and adds the token
 * (`ui/vite.dev-bridge.ts`, development builds only). Commands arrive with `origin = debug-api`.
 * Depends on: ./types, the dev proxy. Called by: ./select (only when `import.meta.env.DEV`).
 * Invariants: same-origin requests only; the token never reaches this code; a dropped event
 * stream is reported to the store as `lagged` after it reconnects, so the model re-reads.
 */
import type { Command, CommandResult, CoreError, CoreEvent } from "../core";
import {
  CoreCallError,
  type CoreClient,
  type TerminalChunkHandler,
  type TerminalStream,
  type UiControlReply,
  type UiControlRequest,
  toCallError,
} from "./types";

const BASE = "/__vw";

/** Event names the Debug API stream uses (`crates/vw-debug-api/src/sse.rs`). */
export const EVENT_NAMES = [
  "snapshot",
  "change",
  "lagged",
  "server",
  "callbackOpened",
  "callbackAnswered",
  "refused",
  "alert",
  "sessionStuck",
  "sessionHolder",
  "debugApi",
  "keyNeeded",
  "keyStatusChanged",
  "vibeInstall",
  "rateLimits",
  "terminal",
  "documentsAdded",
  "documentChanged",
  "changesUpdated",
  "previewChanged",
  "previewShowRequested",
  "browserChanged",
  "pagesChanged",
  "filesChanged",
  "lineCommentsChanged",
  "projectsChanged",
  "worktreesChanged",
  "secretsChanged",
  "dictation",
  "connectorSignIn",
  "marketInstall",
] as const;

/** How often the dev client polls a terminal's output, and how much it reads at once. */
const TERMINAL_POLL_MS = 50;
const TERMINAL_READ_BYTES = 256 * 1024;

async function post<T>(path: string, body: unknown): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`${BASE}${path}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch (thrown) {
    throw toCallError(thrown);
  }
  const text = await response.text();
  let json: unknown;
  try {
    json = JSON.parse(text);
  } catch {
    throw toCallError(
      new Error(`the dev proxy answered ${String(response.status)} with a non-JSON body`),
    );
  }
  if (!response.ok) {
    const error = (json as { error?: CoreError }).error;
    throw error === undefined
      ? toCallError(new Error(`HTTP ${String(response.status)}`))
      : new CoreCallError(error);
  }
  return json as T;
}

function unsupported(message: string): Promise<never> {
  return Promise.reject(toCallError(new Error(message)));
}

/** The Debug-API-backed client for browser development. */
export function createDebugApiClient(): CoreClient {
  return {
    kind: "debug-api",
    execute: (command: Command) => post<CommandResult>("/commands", command),
    subscribe(onEvent) {
      const source = new EventSource(`${BASE}/events`);
      let opened = false;
      source.onopen = () => {
        // A reopened stream starts with fresh snapshots; tell the store so it can re-read.
        if (opened) onEvent({ type: "lagged" });
        opened = true;
      };
      for (const name of EVENT_NAMES) {
        source.addEventListener(name, (message) => {
          onEvent(JSON.parse((message as MessageEvent<string>).data) as CoreEvent);
        });
      }
      return Promise.resolve(() => {
        source.close();
      });
    },
    uiControl: {
      async next(waitMs) {
        const answer = await post<{ request: UiControlRequest | null }>("/ui/ui-client/poll", {
          waitMs,
        });
        return answer.request;
      },
      async reply(reply: UiControlReply) {
        await post<{ accepted: boolean }>("/ui/ui-client/reply", reply);
      },
    },
    pickFolder: () => unsupported("the folder picker works in the app, not in browser dev mode"),
    pickDocuments: () =>
      unsupported("the document picker works in the app, not in browser dev mode"),
    pickLibraryFiles: () =>
      unsupported("the library file picker works in the app, not in browser dev mode"),
    exportPage: () =>
      unsupported("saving through the save dialog works in the app, not in browser dev mode"),
    exportSession: () =>
      unsupported("saving through the save dialog works in the app, not in browser dev mode"),
    revealFolder: () => unsupported("showing a folder works in the app, not in browser dev mode"),
    openLink: () => unsupported("opening a link works in the app, not in browser dev mode"),
    openMailLink: () =>
      unsupported("opening a mail link works in the app, not in browser dev mode"),
    notifyOs: () => unsupported("OS notifications work in the app, not in browser dev mode"),
    onHostEvent: () => Promise.resolve(() => undefined),
    cliSharing: () =>
      Promise.reject(
        toCallError(new Error("the sharing switch works in the app, not in browser dev mode")),
      ),
    setDebugApi: () =>
      Promise.reject(
        toCallError(new Error("the Debug API switch works in the app, not in browser dev mode")),
      ),
    debugApiTokenFile: () =>
      Promise.reject(
        toCallError(new Error("the token file is managed in the app, not in browser dev mode")),
      ),
    attachTerminal: (terminalId, onChunk) => Promise.resolve(pollTerminal(terminalId, onChunk)),
    // Updates belong to the installed app: browser dev mode has no update channel.
    updates: {
      subscribe: () => Promise.resolve(false),
      check: () => unsupported("updates work in the app, not in browser dev mode"),
      download: () => unsupported("updates work in the app, not in browser dev mode"),
      install: () => unsupported("updates work in the app, not in browser dev mode"),
      setAutoDownload: () => unsupported("updates work in the app, not in browser dev mode"),
      openReleasePage: () => unsupported("updates work in the app, not in browser dev mode"),
    },
  };
}

/**
 * Browser dev mode has no byte stream: it reads the output as text from the last offset, from
 * the start of the scrollback on. No flow control (dev only); stops on the first error (the
 * terminal was closed).
 */
function pollTerminal(terminalId: string, onChunk: TerminalChunkHandler): TerminalStream {
  // An object, so the loop sees `detach` flip the flag across its awaits.
  const state = { open: true };
  let from = 0;
  const loop = async (): Promise<void> => {
    while (state.open) {
      let text: string;
      try {
        const result = await post<CommandResult>("/commands", {
          type: "terminal",
          request: { op: "read", terminalId, from, maxBytes: TERMINAL_READ_BYTES },
        });
        if (result.type !== "terminal" || result.result.type !== "output") return;
        from = result.result.output.end;
        text = result.result.output.text;
      } catch {
        return;
      }
      // A chunk arriving after `detach` is dropped by the view (it checks its own disposal).
      if (text.length > 0) onChunk(text, () => undefined);
      else await new Promise((resolve) => setTimeout(resolve, TERMINAL_POLL_MS));
    }
  };
  void loop();
  return {
    detach() {
      state.open = false;
    },
  };
}
