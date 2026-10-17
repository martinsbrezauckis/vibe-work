/**
 * The UI's model of the core: server status, the Debug API flag and the sessions, plus the pure
 * reducer that folds `CoreEvent`s and command results into it (Rust owns the session,
 * the UI owns the screen, components read this model only).
 * Depends on: protocol/core types, sessionChanges, sessionSummary. Called by: coreStore.
 * Invariants: reducers are pure; a `snapshot` replaces an entry whole; a change for an unknown
 * session is ignored (and reported by `needsRefresh`); `lagged` keeps the model and waits for the
 * fresh snapshots that follow it by the subscription's rule.
 */
import type {
  CommandResult,
  CoreError,
  CoreEvent,
  CoreStatus,
  DebugApiFlag,
  JsonValue,
  Origin,
  RateLimitsView,
  ServerStatus,
  SessionRowView,
  SessionSnapshot,
} from "../protocol/core";
import { applyChange } from "./sessionChanges";
import { type IndexState, initialIndex, mergeRows } from "./sessionIndex";
import type { SessionEntry, StuckMap } from "./sessionSummary";

/** What the status bar and header say about the Vibe server. */
export type ServerView =
  | { phase: "connecting" }
  | { phase: "unavailable"; reason: string }
  | { phase: "starting" }
  | { phase: "notInstalled"; reason: string }
  | { phase: "running"; name: string; version: string }
  | { phase: "restarting"; attempt: number; retryInMs: number }
  | { phase: "dead"; reason: string }
  | { phase: "stopped" };

/** The whole core model. */
export interface CoreState {
  server: ServerView;
  debugApi: DebugApiFlag;
  /** Vibe Work's version as the core reports it (`status`); `null` until the first status arrives. */
  appVersion: string | null;
  sessions: Readonly<Record<string, SessionEntry>>;
  /** Paging state of the session index (`sessionIndex.ts`). */
  index: IndexState;
  /** Session ids whose patch failed or whose events had a gap: a re-read is due. */
  needsRefresh: readonly string[];
  /** The event stream reported it fell behind (cleared by the next snapshot). */
  lagged: boolean;
  /** Sessions Vibe could not reopen after a restart; the session remains in the list. */
  stuck: StuckMap;
  /** Sessions another process holds: session id -> its pid. Read-only here. */
  held: Readonly<Record<string, number>>;
  /** Whether a foreign holder is another app window or the terminal. */
  heldKind: Readonly<Record<string, string>>;
  /** Which client answered a callback through the core, by callback id (the audit row). */
  answeredBy: Readonly<Record<string, Origin>>;
  /**
   * The latest per-minute rate-limit headroom a Mistral answer reported through the model proxy,
   * or `null` until one did. Use `state/account.ts` `rateLimitsFresh` before showing
   * it as current.
   */
  rateLimits: RateLimitsView | null;
  /**
   * Open cards of subagents, by callback id. A subagent's card names its child session, whose state is not
   * loaded (it has no channel of its own), so the parent's transcript shows it (`subagentCardsOf`). Added by a
   * `callbackOpened` that carries `subagent`, removed by `callbackAnswered`.
   */
  subagentCards: Readonly<Record<string, SubagentCardEntry>>;
}

/** One subagent's open card: the child session to answer for, its parent, Vibe's callback entry. */
export interface SubagentCardEntry {
  sessionId: string;
  parentSessionId: string;
  entry: JsonValue;
}

/** The state before anything has arrived. */
export const initialCore: CoreState = {
  server: { phase: "connecting" },
  debugApi: { enabled: false, reason: "off" },
  appVersion: null,
  sessions: {},
  index: initialIndex,
  needsRefresh: [],
  lagged: false,
  stuck: {},
  held: {},
  heldKind: {},
  answeredBy: {},
  subagentCards: {},
  rateLimits: null,
};

/** Maps the core's server status to the view model. */
export function serverView(status: ServerStatus): ServerView {
  switch (status.state) {
    case "starting":
      return { phase: "starting" };
    case "notInstalled":
      return { phase: "notInstalled", reason: status.reason };
    case "running":
      return { phase: "running", name: status.serverName, version: status.serverVersion };
    case "restarting":
      return { phase: "restarting", attempt: status.attempt, retryInMs: status.retryInMs };
    case "dead":
      return { phase: "dead", reason: status.reason };
    case "stopped":
      return { phase: "stopped" };
  }
}

function entryFromSnapshot(
  snapshot: SessionSnapshot,
  previous: SessionEntry | undefined,
): SessionEntry {
  const state = snapshot.state;
  const object = typeof state === "object" && state !== null && !Array.isArray(state) ? state : {};
  const session = object["session"];
  const accessMode = snapshot.accessMode ?? previous?.accessMode;
  return {
    id: snapshot.sessionId,
    session:
      typeof session === "object" && session !== null && !Array.isArray(session) ? session : {},
    state: object,
    localOnly: snapshot.localOnly,
    ...(accessMode === undefined ? {} : { accessMode }),
    openCallbackIds: previous?.openCallbackIds ?? [],
    // A snapshot carries no runtime: keep the last one.
    ...(previous?.runtime === undefined ? {} : { runtime: previous.runtime }),
  };
}

function withSession(state: CoreState, entry: SessionEntry): CoreState {
  return { ...state, sessions: { ...state.sessions, [entry.id]: entry } };
}

function without(sessions: CoreState["sessions"], id: string): CoreState["sessions"] {
  return Object.fromEntries(Object.entries(sessions).filter(([key]) => key !== id));
}

/** Folds one core event into the model. */
export function applyEvent(state: CoreState, event: CoreEvent): CoreState {
  switch (event.type) {
    case "snapshot":
      return {
        ...withSession(
          state,
          entryFromSnapshot(event.snapshot, state.sessions[event.snapshot.sessionId]),
        ),
        lagged: false,
        needsRefresh: event.snapshot.resyncPending
          ? [...new Set([...state.needsRefresh, event.snapshot.sessionId])]
          : state.needsRefresh.filter((id) => id !== event.snapshot.sessionId),
      };
    case "change": {
      const change = event.change;
      if (change.kind === "notice") return state;
      if (change.kind === "removed")
        return { ...state, sessions: without(state.sessions, change.sessionId) };
      if (change.kind === "handedOff")
        return { ...state, sessions: without(state.sessions, change.oldSessionId) };
      const entry = state.sessions[change.sessionId];
      if (entry === undefined) return state;
      const applied = applyChange(entry, change);
      const next = withSession(state, applied.entry);
      return applied.ok
        ? next
        : { ...next, needsRefresh: [...new Set([...state.needsRefresh, entry.id])] };
    }
    case "lagged":
      return { ...state, lagged: true };
    case "server":
      return { ...state, server: serverView(event.status) };
    case "debugApi":
      return { ...state, debugApi: event.flag };
    case "sessionStuck": {
      const stuck = Object.fromEntries(
        Object.entries(state.stuck).filter(([id]) => id !== event.sessionId),
      );
      if (event.stuck) stuck[event.sessionId] = event.continuedIn;
      return { ...state, stuck };
    }
    case "sessionHolder": {
      const held = Object.fromEntries(
        Object.entries(state.held).filter(([id]) => id !== event.sessionId),
      );
      if (event.holderPid !== null) held[event.sessionId] = event.holderPid;
      const heldKind = Object.fromEntries(
        Object.entries(state.heldKind).filter(([id]) => id !== event.sessionId),
      );
      if (event.holderPid !== null) heldKind[event.sessionId] = event.holderKind ?? "window";
      return { ...state, held, heldKind };
    }
    case "callbackAnswered": {
      const subagentCards = Object.fromEntries(
        Object.entries(state.subagentCards).filter(([id]) => id !== event.callbackId),
      );
      return {
        ...state,
        answeredBy: { ...state.answeredBy, [event.callbackId]: event.origin },
        subagentCards,
      };
    }
    case "callbackOpened":
      if (event.subagent === undefined) return state;
      return {
        ...state,
        subagentCards: {
          ...state.subagentCards,
          [event.callbackId]: {
            sessionId: event.sessionId,
            parentSessionId: event.subagent.parentSessionId,
            entry: event.subagent.entry,
          },
        },
      };
    // Terminals have their own store (features/terminal), fed through `CoreController.listen`.
    // The setup and unlock UI handles key and vault events (`keyNeeded`, `keyStatusChanged`).
    // The Changes tab follows `changesUpdated` itself (features/changes/store.ts).
    // Documents (`state/documents.ts`), preview and Pages each have stores fed through `listen`.
    // The connectors store follows sign-ins (`state/connectorSignIn.ts`); the marketplace store
    // follows `marketInstall` (`state/market.ts`). The project groups store follows
    // `projectsChanged` (`state/projectGroups.ts`); Settings › Project secrets follows
    // `secretsChanged` itself. The Vibe install has its own store (`state/vibeInstall.ts`).
    case "rateLimits":
      return { ...state, rateLimits: event.limits };
    case "refused":
    case "alert":
    case "terminal":
    case "keyNeeded":
    case "keyStatusChanged":
    case "vibeInstall":
    case "documentsAdded":
    case "documentChanged":
    case "changesUpdated":
    case "previewChanged":
    case "previewShowRequested":
    case "browserChanged":
    case "pagesChanged":
    case "filesChanged":
    case "lineCommentsChanged":
    case "projectsChanged":
    case "worktreesChanged":
    case "gitBrowserChanged":
    case "secretScan":
    case "secretsChanged":
    case "dictation":
    case "connectorSignIn":
    case "marketInstall":
      return state;
  }
}

/**
 * The subagent cards the parent's transcript shows: those still open in Vibe's view of the parent (a
 * `childSessions` row of that child is blocked on that card), so a Stop or an answer given elsewhere hides them.
 */
export function subagentCardsOf(
  cards: CoreState["subagentCards"],
  parentSessionId: string,
  childSessions: JsonValue | undefined,
): readonly (SubagentCardEntry & { callbackId: string })[] {
  const rows = Array.isArray(childSessions) ? childSessions : [];
  const waiting = new Set(
    rows.flatMap((row) => {
      if (row === null || typeof row !== "object" || Array.isArray(row)) return [];
      const status = row["status"];
      if (status === null || typeof status !== "object" || Array.isArray(status)) return [];
      const id = row["id"] ?? row["sessionId"];
      const callbackId = status["callbackId"];
      return typeof id === "string" && typeof callbackId === "string"
        ? [`${id}\u0000${callbackId}`]
        : [];
    }),
  );
  return Object.entries(cards)
    .filter(
      ([callbackId, card]) =>
        card.parentSessionId === parentSessionId &&
        waiting.has(`${card.sessionId}\u0000${callbackId}`),
    )
    .map(([callbackId, card]) => ({ ...card, callbackId }));
}

/** Folds a `status` command result (startup and reconnect) into the model. */
export function applyStatus(state: CoreState, status: CoreStatus): CoreState {
  const stuck: Record<string, string | null> = {};
  for (const s of status.channels.stuck) stuck[s.sessionId] = s.continuedIn;
  return {
    ...state,
    server: serverView(status.server.status),
    debugApi: status.debugApi,
    appVersion: status.appVersion,
    rateLimits: status.rateLimits,
    stuck,
  };
}

/** Merges a `sessions` listing: unknown ids become listing-only rows, known ones keep their state. */
export function applyListing(state: CoreState, rows: readonly SessionRowView[]): CoreState {
  return { ...state, sessions: mergeRows(state.sessions, rows) };
}

/** Folds the result of any command that carries a session or a listing. */
export function applyResult(state: CoreState, result: CommandResult): CoreState {
  switch (result.type) {
    case "sessions":
      return applyListing(state, result.rows);
    case "session":
      return applyEvent(state, { type: "snapshot", snapshot: result.snapshot });
    case "status":
      return applyStatus(state, result.status);
    default:
      return state;
  }
}

/** The error a command gets while Vibe is not installed yet (`serverUnavailable` / `notInstalled`). */
export function notInstalled(error: CoreError): boolean {
  return error.code === "serverUnavailable" && error.serverCode === "notInstalled";
}

/** The model while Vibe is not installed yet: the setup screen offers the install. */
export function applyNotInstalled(state: CoreState, error: CoreError): CoreState {
  return { ...state, server: { phase: "notInstalled", reason: error.message } };
}

/** The model after the core could not be reached (startup failure or a dead bridge). */
export function applyUnavailable(state: CoreState, error: CoreError): CoreState {
  return { ...state, server: { phase: "unavailable", reason: error.message } };
}
