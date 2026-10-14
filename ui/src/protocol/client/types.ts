/**
 * The typed client interface over the core API: the only way the UI talks to Rust.
 * Two implementations: Tauri IPC (the real app) and the Debug API through the dev proxy
 * (a normal browser against a real core); tests use a scripted fake.
 * Depends on: protocol/core (generated types). Called by: state/coreStore, state/uiControl, main.
 * Invariants: every failure is a `CoreCallError` carrying a `CoreError` (never a raw string); the
 * client holds no state of its own.
 */
import type {
  Command,
  CommandResult,
  CoreError,
  CoreEvent,
  DocumentRefusal,
  DocumentView,
  PagesResult,
  SessionExportView,
} from "../core";

/** What the native document picker added (`vw_host::DocumentsPicked`). Never a path. */
export interface DocumentsPicked {
  added: DocumentView[];
  refused: DocumentRefusal[];
}

/** A failed core call. */
export class CoreCallError extends Error {
  /** The structured error from the core (or synthesized for transport failures). */
  readonly error: CoreError;

  constructor(error: CoreError) {
    super(error.message);
    this.name = "CoreCallError";
    this.error = error;
  }
}

/** One UI-control request from the Debug API (`POST /ui/<name>`), waiting for this UI. */
export interface UiControlRequest {
  id: string;
  name: string;
  body: unknown;
  /**
   * The request's capability: commands run for this request pass it, and the host
   * accepts the Debug API origin only while this exact request is pending. Absent from transports that do not
   * need it (a browser UI already talks to the Debug API itself).
   */
  capability?: string;
}

/** The answer to one UI-control request. */
export type UiControlReply =
  | { id: string; ok: true; value: unknown }
  | { id: string; ok: false; error: { code: CoreError["code"]; message: string } };

/** How the UI receives and answers Debug API `ui/*` requests. */
export interface UiControlTransport {
  /** Waits up to `waitMs` for the next request; `null` when none arrived. */
  next(waitMs: number): Promise<UiControlRequest | null>;
  /** Answers one request. */
  reply(reply: UiControlReply): Promise<void>;
}

/** Something the host (not Vibe) tells the UI; mirrors `vw_host::HostEvent`. */
export type HostEvent =
  | { type: "openSession"; sessionId: string; notificationId?: number }
  | { type: "openUpdates" }
  | { type: "browserSaveContext"; sessionId: string };

/** Where the update flow is (mirrors `vw_host::update_flow::Phase`). */
export type UpdatePhase =
  | { type: "idle" }
  | { type: "checking" }
  | { type: "available"; version: string; notes: string | null }
  | {
      type: "downloading";
      version: string;
      notes: string | null;
      received: number;
      total: number | null;
    }
  | { type: "ready"; version: string; notes: string | null }
  | { type: "installing"; version: string }
  | { type: "restarting"; version: string }
  | {
      type: "failed";
      stage: "check" | "download" | "install";
      error: string;
      version: string | null;
    };

/** What the update flow shows (mirrors `vw_host::update_flow::UpdateView`; a private Rust test pins the JSON). */
export interface UpdateView {
  current: string;
  /** `inApp`: download, ask, install here. `linkOnly`: a package-manager install; only a link is offered. */
  mode: "inApp" | "linkOnly";
  /** The "Download updates automatically" setting. */
  autoDownload: boolean;
  /** The offered version's release page: always the official repository's tag, built by the host (never a link from the update manifest). */
  releasePage: string | null;
  /** Unix seconds of the last finished check. */
  lastChecked: number | null;
  phase: UpdatePhase;
}

/** The update commands. Absent in browser dev mode (`subscribe` then reports no channel). */
export interface UpdatesClient {
  /** Starts the view stream (the current view first). `false`: this build has no update channel. */
  subscribe(onView: (view: UpdateView) => void): Promise<boolean>;
  /** Checks now. Resolves when the check (and an automatic download) finished. */
  check(): Promise<void>;
  /**
   * Downloads the offered update. `origin: "debug-api"` with the served request's `capability` marks it as the
   * Debug API's (its request is the consent); the host refuses the mark without a live request.
   */
  download(options?: ExecuteOptions): Promise<void>;
  /** "Install and restart"; same origin rule as `download`. The core refuses without a recorded agreement. */
  install(options?: ExecuteOptions): Promise<void>;
  /** Saves "Download updates automatically". */
  setAutoDownload(enabled: boolean): Promise<void>;
  /** Opens the offered version's release page in the system browser. */
  openReleasePage(): Promise<void>;
}

/** What the UI asks the host to show as an OS notification (`vw_host::NotifyRequest`). */
export interface OsNotification {
  sessionId: string;
  /** In-app history entry to select on activation; omitted by older callers. */
  notificationId?: number;
  title: string;
  body: string;
}

/** Per-call options of `CoreClient.execute`. */
export interface ExecuteOptions {
  /**
   * `debug-api`: run for a Debug API request. `window`: the client's own origin even while a Debug API `ui/*`
   * request is being served (what the view sends by itself, such as a terminal's focus report).
   * Omitted = the client's own origin, marked as the Debug API's by the core controller while a request is served.
   */
  origin?: "debug-api" | "window";
  /**
   * With `origin: "debug-api"` in the app: the capability of the `ui/*` request being served (filled in by the
   * core controller). The host refuses a Debug API claim without a live one.
   */
  capability?: string;
}

/**
 * One terminal's output stream. Chunks arrive in order: first the scrollback replay,
 * then live output. The view calls `done()` for each chunk once it has rendered it; that is the
 * flow control that keeps a 50 MB `cat` from flooding the window.
 */
export interface TerminalStream {
  /** Stops the stream; the terminal keeps running and buffering in the core. */
  detach(): void;
}

/** Receives one output chunk; `done` acknowledges it after rendering. */
export type TerminalChunkHandler = (chunk: Uint8Array | string, done: () => void) => void;

/** The core as the UI sees it. */
export interface CoreClient {
  /** Which transport this is (shown in diagnostics and tests). */
  readonly kind: "tauri" | "debug-api" | "fake";
  /**
   * Executes one command with `origin = ui` (Tauri) or `debug-api` (browser dev mode). `origin:
   * "debug-api"` with `capability` runs a Tauri call for the Debug API `ui/*` request being served.
   * That origin acts as the user, so the host accepts it only while that exact request is pending.
   */
  execute(command: Command, options?: ExecuteOptions): Promise<CommandResult>;
  /**
   * Starts the event stream: first a snapshot of every loaded session, then changes. A second
   * call replaces the first stream. Returns the function that stops listening.
   */
  subscribe(onEvent: (event: CoreEvent) => void): Promise<() => void>;
  /** The `ui/*` request transport. */
  readonly uiControl: UiControlTransport;
  /**
   * Opens the native folder picker (Rust owns it, no path comes from the webview), registers the
   * choice with the gate and returns it; `null` when cancelled. Unsupported in browser dev mode.
   */
  pickFolder(): Promise<string | null>;
  /**
   * Opens the native file picker for documents (Rust owns it; no path comes from or goes to the
   * webview) and adds the chosen files to the core. Unsupported in browser dev mode.
   */
  pickDocuments(): Promise<DocumentsPicked>;
  /**
   * Opens the native file picker for a library (Rust owns it, the webview names only the library) and
   * uploads the chosen files to Mistral. Answers the `libraries` result (added, refused). Unsupported in browser dev
   * mode (use the `upload` request with the bytes instead).
   */
  pickLibraryFiles(libraryId: string): Promise<CommandResult>;
  /**
   * "Share as one file": opens the native save dialog (Rust owns it; the webview names only the session
   * and the page) and writes the page there as one self-contained HTML file. `null` when the user cancelled.
   * Unsupported in browser dev mode (the Debug API saves with `exportPage {destination}` instead).
   */
  exportPage(sessionId: string, path: string): Promise<PagesResult | null>;
  /**
   * "Export as Markdown": opens the native save dialog (Rust owns it; the webview names only the session)
   * and writes the whole session there. `null` when the user cancelled. Unsupported in browser dev mode (the Debug
   * API exports with `sessionTools {op: "export", destination}` instead).
   */
  exportSession(sessionId: string): Promise<SessionExportView | null>;
  /** Shows a session's folder in the file manager (the host refuses other paths). */
  revealFolder(path: string): Promise<void>;
  /**
   * Opens a web link (http/https only) in the system browser, after the user confirmed it in the
   * link dialog. The host validates the URL again; the main window never navigates.
   * Unsupported in browser dev mode.
   */
  openLink(url: string): Promise<void>;
  /**
   * Opens a `mailto:` link to one plain address in the system's mail program (Settings › About contact).
   * The host refuses anything else. Unsupported in browser dev mode.
   */
  openMailLink(url: string): Promise<void>;
  /** Shows an OS notification; a click arrives through `onHostEvent`. Unsupported in dev mode. */
  notifyOs(notification: OsNotification): Promise<void>;
  /** Starts the host event stream; returns the function that stops listening. */
  onHostEvent(onEvent: (event: HostEvent) => void): Promise<() => void>;
  /**
   * The CLI session-sharing switch: saves `share` first when given, then returns the switch,
   * whether the Vibe CLI's home exists and both Vibe versions. A change applies at the next start.
   * Unsupported where the host cannot.
   */
  cliSharing(share?: boolean): Promise<CliSharingView>;
  /** The update flow. */
  readonly updates: UpdatesClient;
  /** Turns the Debug API on or off and remembers it; unsupported where the host cannot. */
  setDebugApi(enabled: boolean): Promise<void>;
  /**
   * The Debug API token file's PATH (for Settings), regenerating the token first when `rotate` is
   * true (the old token stops working at once). Never the token itself: it does not cross the
   * bridge. Unsupported where the host cannot.
   */
  debugApiTokenFile(rotate: boolean): Promise<string>;
  /**
   * Streams a terminal's output into `onChunk` (scrollback first). Tauri: raw bytes on a Channel
   * with acknowledgements; browser dev mode: polls the `read` command (no flow control).
   */
  attachTerminal(terminalId: string, onChunk: TerminalChunkHandler): Promise<TerminalStream>;
}

/** Normalizes anything a transport throws into a `CoreCallError`. */
export function toCallError(thrown: unknown): CoreCallError {
  if (thrown instanceof CoreCallError) return thrown;
  if (typeof thrown === "object" && thrown !== null && "code" in thrown && "message" in thrown) {
    const e = thrown as Partial<CoreError>;
    return new CoreCallError({
      code: e.code ?? "internal",
      message: String(e.message),
      method: e.method ?? null,
      summary: e.summary ?? null,
      serverCode: e.serverCode ?? null,
    });
  }
  const message = thrown instanceof Error ? thrown.message : String(thrown);
  return new CoreCallError({
    code: "internal",
    message,
    method: null,
    summary: null,
    serverCode: null,
  });
}

/** What `cli_sharing` answers (`vw_host::cli_sharing::CliSharingView`). */
export interface CliSharingView {
  share: boolean;
  cliHomeFound: boolean;
  cliVersion: string | null;
  bundledVersion: string | null;
  versionMismatch: boolean;
}
