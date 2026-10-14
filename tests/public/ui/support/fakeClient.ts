/**
 * A scripted `CoreClient` for UI tests: records every command, answers from a table, lets a test
 * push core events as if they came from Rust, and exposes the UI-control transport as a queue.
 * Synthetic data only. Depends on: ui/src/protocol/client/types.
 */
import { vi } from "vitest";

import type {
  CliSharingView,
  CoreClient,
  DocumentsPicked,
  HostEvent,
  OsNotification,
  TerminalChunkHandler,
  UpdateView,
  UiControlReply,
  UiControlRequest,
} from "../../../../ui/src/protocol/client/types";
import { CoreCallError } from "../../../../ui/src/protocol/client/types";
import type {
  Command,
  CommandResult,
  CoreError,
  CoreEvent,
  CoreStatus,
  PagesResult,
  SessionExportView,
} from "../../../../ui/src/protocol/core";

export const RUNNING_STATUS: CoreStatus = {
  appVersion: "0.0.0",
  vibeVersion: "9.9.9",
  uptimeMs: 1,
  server: {
    status: {
      state: "running",
      generation: 1,
      serverName: "fake-vibe",
      serverVersion: "9.9.9",
      needsResume: [],
      recovering: [],
    },
    pid: 1,
    stderrTail: [],
  },
  debugApi: { enabled: false, reason: "off" },
  loadedSessions: [],
  channels: { open: [], foreignDropped: 0, resyncReads: 0, stuck: [] },
  rateLimits: null,
};

export interface FakeClient extends CoreClient {
  commands: Command[];
  /** The origin each command was sent with (same order as `commands`): "debug-api" when the UI lowered it. */
  origins: string[];
  /** Sets the answer for a command type (a result, or a CoreError to reject with). */
  answer(type: Command["type"], result: CommandResult | CoreError): void;
  /** Answers a command type from a function of the command (paging, per-method replies). */
  answerWith(
    type: Command["type"],
    reply: (command: Command) => CommandResult | CoreError,
  ): void;
  /** Delivers an event to the subscriber, as the Rust side would. */
  emit(event: CoreEvent): void;
  /** Queues a UI-control request for the UI's loop to pick up. */
  request(request: UiControlRequest): void;
  replies: UiControlReply[];
  /** Delivers a host event (an OS notification was clicked). */
  emitHost(event: HostEvent): void;
  /** Notifications the UI asked the host to show. */
  notifications: OsNotification[];
  pickFolder: ReturnType<typeof vi.fn<() => Promise<string | null>>>;
  /** The native document picker: answers what `pickedDocuments` holds (default: nothing). */
  pickDocuments: ReturnType<typeof vi.fn<() => Promise<DocumentsPicked>>>;
  pickedDocuments: DocumentsPicked;
  /** The native library picker answers an empty upload unless a test replaces it. */
  pickLibraryFiles: ReturnType<
    typeof vi.fn<(libraryId: string) => Promise<CommandResult>>
  >;
  exportPage: ReturnType<
    typeof vi.fn<
      (sessionId: string, path: string) => Promise<PagesResult | null>
    >
  >;
  /** the save dialog of "Export as Markdown" (default: cancelled). */
  exportSession: ReturnType<
    typeof vi.fn<(sessionId: string) => Promise<SessionExportView | null>>
  >;
  revealFolder: ReturnType<typeof vi.fn<(path: string) => Promise<void>>>;
  openLink: ReturnType<typeof vi.fn<(url: string) => Promise<void>>>;
  openMailLink: ReturnType<typeof vi.fn<(url: string) => Promise<void>>>;
  cliSharing: ReturnType<
    typeof vi.fn<(share?: boolean) => Promise<CliSharingView>>
  >;
  setDebugApi: ReturnType<typeof vi.fn<(enabled: boolean) => Promise<void>>>;
  debugApiTokenFile: ReturnType<
    typeof vi.fn<(rotate: boolean) => Promise<string>>
  >;
  /**
   * The update flow . Default: this build has no update channel (`subscribe` answers false). A test
   * sets `updateChannel = true` before the app starts, then pushes views with `emitUpdate`.
   */
  updateChannel: boolean;
  emitUpdate(view: UpdateView): void;
  /** Every update call, with the origin the UI sent (`ui` unless it lowered it to `debug-api`). */
  updateCalls: { name: string; origin: string; arg?: unknown }[];
  /** Makes the next call of this name reject with this error code and message. */
  updateFailure: Map<string, { code: CoreError["code"]; message: string }>;
  /** Output handlers of attached terminal views, by terminal id . */
  terminalStreams: Map<string, TerminalChunkHandler>;
  /** Terminal ids whose stream was detached. */
  detached: string[];
}

export function fakeClient(kind: CoreClient["kind"] = "fake"): FakeClient {
  type Answer =
    | CommandResult
    | CoreError
    | ((command: Command) => CommandResult | CoreError);
  const answers = new Map<string, Answer>([
    ["readStatus", { type: "status", status: RUNNING_STATUS }],
    [
      "viewSession",
      {
        type: "reply",
        result: null,
        clientUserMessageId: null,
        userEntryIds: [],
      },
    ],
    [
      "listSessions",
      { type: "sessions", rows: [], nextCursor: null, continueSessionId: null },
    ],
    // nothing is left running unless a test scripts it.
    [
      "processes",
      (command: Command): CommandResult | CoreError => {
        if (command.type !== "processes" || command.request.op === "stop")
          return {
            code: "notFound",
            message: "that process is not on the list",
            method: null,
            summary: null,
            serverCode: null,
          };
        return command.request.op === "leftRunning"
          ? { type: "processes", result: { kind: "leftRunning", items: [] } }
          : {
              type: "processes",
              result: {
                kind: "afterDelete",
                asked: false,
                stopped: [],
                failed: [],
                left: [],
              },
            };
      },
    ],
  ]);
  let handler: ((event: CoreEvent) => void) | null = null;
  let hostHandler: ((event: HostEvent) => void) | null = null;
  let updateHandler: ((view: UpdateView) => void) | null = null;
  const updateCall = (
    name: string,
    origin: string,
    arg?: unknown,
  ): Promise<void> => {
    client.updateCalls.push({
      name,
      origin,
      ...(arg === undefined ? {} : { arg }),
    });
    const failure = client.updateFailure.get(name);
    if (failure !== undefined) {
      client.updateFailure.delete(name);
      return Promise.reject(
        new CoreCallError({
          ...failure,
          method: null,
          summary: null,
          serverCode: null,
        }),
      );
    }
    return Promise.resolve();
  };
  const queue: UiControlRequest[] = [];
  const waiters: ((r: UiControlRequest | null) => void)[] = [];
  const client: FakeClient = {
    kind,
    commands: [],
    origins: [],
    replies: [],
    notifications: [],
    pickFolder: vi.fn<() => Promise<string | null>>(() =>
      Promise.resolve("/work/picked"),
    ),
    pickedDocuments: { added: [], refused: [] },
    pickDocuments: vi.fn<() => Promise<DocumentsPicked>>(() =>
      Promise.resolve(client.pickedDocuments),
    ),
    pickLibraryFiles: vi.fn<(libraryId: string) => Promise<CommandResult>>(() =>
      Promise.resolve({
        type: "libraries",
        result: { kind: "uploaded", added: [], refused: [] },
      } as CommandResult),
    ),
    exportPage: vi.fn<
      (sessionId: string, path: string) => Promise<PagesResult | null>
    >(() => Promise.resolve(null)),
    exportSession: vi.fn<
      (sessionId: string) => Promise<SessionExportView | null>
    >(() => Promise.resolve(null)),
    revealFolder: vi.fn<(path: string) => Promise<void>>(() =>
      Promise.resolve(),
    ),
    openLink: vi.fn<(url: string) => Promise<void>>(() => Promise.resolve()),
    openMailLink: vi.fn<(url: string) => Promise<void>>(() =>
      Promise.resolve(),
    ),
    notifyOs(notification) {
      client.notifications.push(notification);
      return Promise.resolve();
    },
    onHostEvent(onEvent) {
      hostHandler = onEvent;
      return Promise.resolve(() => {
        hostHandler = null;
      });
    },
    emitHost(event) {
      hostHandler?.(event);
    },
    answer: (type, result) => void answers.set(type, result),
    answerWith: (type, reply) => void answers.set(type, reply),
    execute(command, options) {
      client.commands.push(command);
      client.origins.push(options?.origin ?? "ui");
      const scripted = answers.get(command.type);
      const answer =
        typeof scripted === "function" ? scripted(command) : scripted;
      if (answer === undefined)
        return Promise.reject(
          new CoreCallError({
            code: "internal",
            message: `no scripted answer for ${command.type}`,
            method: null,
            summary: null,
            serverCode: null,
          }),
        );
      return "type" in answer
        ? Promise.resolve(answer)
        : Promise.reject(new CoreCallError(answer));
    },
    subscribe(onEvent) {
      handler = onEvent;
      return Promise.resolve(() => {
        handler = null;
      });
    },
    emit(event) {
      if (handler === null) throw new Error("nobody is subscribed");
      handler(event);
    },
    uiControl: {
      next: (waitMs) =>
        new Promise((resolve) => {
          const ready = queue.shift();
          if (ready !== undefined) resolve(ready);
          else {
            waiters.push(resolve);
            // A timed-out waiter leaves the list, so a later request is not handed to it and lost.
            setTimeout(
              () => {
                const at = waiters.indexOf(resolve);
                if (at >= 0) waiters.splice(at, 1);
                resolve(null);
              },
              Math.min(waitMs, 20),
            );
          }
        }),
      reply(reply) {
        client.replies.push(reply);
        return Promise.resolve();
      },
    },
    request(request) {
      const waiter = waiters.shift();
      if (waiter !== undefined) waiter(request);
      else queue.push(request);
    },
    cliSharing: vi.fn<(share?: boolean) => Promise<CliSharingView>>((share) =>
      Promise.resolve({
        share: share ?? true,
        cliHomeFound: true,
        cliVersion: null,
        bundledVersion: "2.26.0",
        versionMismatch: false,
      }),
    ),
    setDebugApi: vi.fn<(enabled: boolean) => Promise<void>>(() =>
      Promise.resolve(),
    ),
    debugApiTokenFile: vi.fn<(rotate: boolean) => Promise<string>>(() =>
      Promise.resolve("/data/debug-api/debug-api.token"),
    ),
    updateChannel: false,
    updateCalls: [],
    updateFailure: new Map(),
    emitUpdate(view) {
      updateHandler?.(view);
    },
    updates: {
      subscribe(onView) {
        updateHandler = onView;
        return Promise.resolve(client.updateChannel);
      },
      check: () => updateCall("check", "ui"),
      download: (options) => updateCall("download", options?.origin ?? "ui"),
      install: (options) => updateCall("install", options?.origin ?? "ui"),
      setAutoDownload: (enabled) =>
        updateCall("setAutoDownload", "ui", enabled),
      openReleasePage: () => updateCall("openReleasePage", "ui"),
    },
    terminalStreams: new Map(),
    detached: [],
    attachTerminal(terminalId, onChunk) {
      client.terminalStreams.set(terminalId, onChunk);
      return Promise.resolve({
        detach() {
          client.terminalStreams.delete(terminalId);
          client.detached.push(terminalId);
        },
      });
    },
  };
  return client;
}

/** A `PublicSessionState` (loosely typed, as the core passes it) for one session. */
export function sessionState(
  id: string,
  status: unknown,
  extra: Record<string, unknown> = {},
) {
  return {
    eventId: 1,
    session: {
      id,
      title: `Session ${id}`,
      status,
      cwd: "/work/alpha",
      updatedAt: 100,
      createdAt: 1,
      ...extra,
    },
    history: [],
    turns: [],
    activeCallbacks: [],
  };
}

export function snapshot(
  id: string,
  status: unknown = { type: "idle" },
  extra: Record<string, unknown> = {},
): CoreEvent {
  return {
    type: "snapshot",
    snapshot: {
      sessionId: id,
      state: sessionState(id, status, extra),
      lastEventId: 1,
      localOnly: false,
      resyncPending: false,
      interruptedTurns: [],
      anomalies: {
        duplicateEvents: 0,
        eventGaps: 0,
        reannouncedEntries: 0,
        completedEntryUpdates: 0,
        patchFailures: 0,
        malformedPayloads: 0,
        inconsistentEnvelopes: 0,
        interruptedTurns: 0,
      },
    },
  } as CoreEvent;
}
