/**
 * What the list column and status bar need to know about one session, derived from the session
 * object Vibe reports (`PublicSession`). Components read these, they never dig into raw JSON.
 * Depends on: protocol/core types. Called by: the list column, the status bar, the UI-control
 * `state` route, the palette, the list model (`features/sessions/listModel.ts`).
 * Invariants: tolerant of the two shapes `status` has been seen in (`{type: "running"}` from Vibe,
 * a bare string from the scripted test server); an unknown status is shown as idle, never thrown.
 */
import { TEXT } from "../vocabulary";
import { pendingHistoryAsks } from "../features/transcript/model/protectedChange";
import type { JsonValue } from "../protocol/core";
import type { TurnRetry } from "./turnRetry";

/** The badge shown next to a session, including failure state. */
export type Badge = "run" | "input" | "done" | "idle" | "failed" | "stuck";

/** Human text for each badge (also the accessible name). */
export const BADGE_LABEL: Record<Badge, string> = {
  run: "Running",
  input: "Needs input",
  done: "Complete",
  idle: "Idle",
  failed: "Failed",
  stuck: "Vibe can't reopen this chat right now",
};

/** Stuck sessions (the core's `sessionStuck` events): id -> the session it was continued in. */
export type StuckMap = Readonly<Record<string, string | null>>;

/** One row of the session list. */
export interface SessionSummary {
  id: string;
  title: string;
  /** Last path segment of the working folder; "No folder" when Vibe reports none. */
  project: string;
  cwd: string | null;
  badge: Badge;
  /** Something happened since the user last opened it. */
  unread: boolean;
  updatedAt: number;
  /** Vibe's `bumpedAt` (last real activity), falling back to `updatedAt`: the list sort key. */
  bumpedAt: number;
  archived: boolean;
  /** Loaded without a turn and not working: Vibe has no restorable chat to archive yet. */
  neverRan: boolean;
  /** `pinnedAt` is set. */
  pinned: boolean;
  /** `parentSessionId` of a child (subagent) session; `null` for a top-level one. */
  parentId: string | null;
  /** First user message as Vibe reports it (search text, may be empty). */
  preview: string;
  model: string | null;
  /** Extra folders (`workspaceRoots`) besides `cwd`. */
  extraFolders: readonly string[];
  /** Created here and not listed by Vibe yet (marked "local" until it is). */
  localOnly: boolean;
  /** Vibe could not reopen it after a restart: commands are refused until Try again works. */
  stuck: boolean;
  /** The session "Continue in a new session" started for it, if any. */
  continuedIn: string | null;
}

/** A session as the core store holds it. */
export interface SessionEntry {
  id: string;
  /** `PublicSession`: always present, from a snapshot or from a listing. */
  session: Record<string, JsonValue | undefined>;
  /** The full `PublicSessionState` once a snapshot arrived; `null` for listing-only rows. */
  state: Record<string, JsonValue | undefined> | null;
  localOnly: boolean;
  /** The core's resolved access mode, available even before the session has a runtime. */
  accessMode?: string;
  /** Ownership derived by the core from the parent's childSessions, never from ancestry alone. */
  ownedByParent?: boolean;
  /** Callbacks (approvals, questions) currently open, from `callbacksChanged`. */
  openCallbackIds: readonly string[];
  /** Open cards hosted for owned subagents, independent of the parent turn status. */
  childCallbackIds?: readonly string[];
  /**
   * The latest `RuntimeSnapshot` (models, profiles, active agent) from `runtimeUpdated` or a reply that
   * carried one (the composer's pickers); absent until one arrived.
   */
  runtime?: JsonValue;
  /** The running turn's model-request retry (`turnRetry.ts`); absent when there is none. */
  retry?: TurnRetry;
}

function str(value: JsonValue | undefined): string | null {
  return typeof value === "string" && value.trim() !== "" ? value : null;
}

/** The status tag of a session object (`running`, `blocked`, `idle`, ...). */
export function statusType(session: SessionEntry["session"]): string {
  const status = session["status"];
  if (typeof status === "string") return status;
  if (typeof status === "object" && status !== null && !Array.isArray(status)) {
    return str(status["type"]) ?? "idle";
  }
  return "idle";
}

/** What a Temporary chat's own folder is called in the window: its random name means nothing to the person. */
export const TEMPORARY_CHAT = "Temporary chat";

/** The one sidebar group every Temporary chat in a folder of its own sits in (the core's project has this name too). */
export const TEMPORARY_CHATS = "Temporary chats";

/**
 * True for a folder Vibe Work made for a Temporary chat: a `chat-` + 16 hex digits folder directly in a
 * `temporary-folders` folder (the core's `vibe-work-temporary-folders-<process>` in the system temp folder counts
 * too). A folder the person chose never matches, so it keeps its own name and path.
 */
export function isTemporaryFolder(cwd: string | null): boolean {
  if (cwd === null) return false;
  const parts = cwd.split(/[\\/]+/).filter((p) => p !== "");
  const name = parts[parts.length - 1] ?? "";
  const parent = parts[parts.length - 2] ?? "";
  return /^chat-[0-9a-f]{16}$/i.test(name) && /(^|-)temporary-folders(-\d+)?$/i.test(parent);
}

/** The folder name used to group sessions into projects ("Temporary chat" for a Temporary chat's own folder). */
export function projectOf(cwd: string | null): string {
  if (cwd === null) return "No folder";
  if (isTemporaryFolder(cwd)) return TEMPORARY_CHAT;
  const parts = cwd.split(/[\\/]+/).filter((p) => p !== "");
  return parts[parts.length - 1] ?? "No folder";
}

/**
 * The badge for a session entry (`stuck` wins: nothing can run until the user acts). `waiting`: the sessions that
 * show a waiting app-owned prompt (a card, or a window of its own), worked out by the caller from the core's asks.
 */
export function badgeOf(
  entry: SessionEntry,
  stuck: StuckMap = {},
  waiting: ReadonlySet<string> = new Set(),
): Badge {
  if (entry.id in stuck) return "stuck";
  if (waiting.has(entry.id)) return "input";
  if ((entry.childCallbackIds?.length ?? 0) > 0) return "input";
  if (pendingHistoryAsks(entry.state?.["history"]).length > 0) return "input";
  const type = statusType(entry.session);
  // An open callback only counts while the session is working: the session's status decides, and an
  // idle session cannot be waiting for anything.
  const working = type !== "idle" && type !== "failed" && type !== "archived";
  if (type === "blocked" || (working && entry.openCallbackIds.length > 0)) return "input";
  if (type === "running") return "run";
  if (type === "failed") return "failed";
  return entry.session["isUnseen"] === true ? "done" : "idle";
}

/** The session's visible name while Vibe is still preparing its title ("Temporary chat" for an untitled one). */
export function sessionTitle(session: SessionEntry["session"] | undefined): string {
  return (
    str(session?.["title"]) ??
    str(session?.["preview"]) ??
    (isTemporaryFolder(str(session?.["cwd"])) ? TEMPORARY_CHAT : TEXT.untitledSession)
  );
}

/** Derives the row for one entry (`waiting`: see `badgeOf`). */
export function summarize(
  entry: SessionEntry,
  stuck: StuckMap = {},
  waiting: ReadonlySet<string> = new Set(),
): SessionSummary {
  const s = entry.session;
  const cwd = str(s["cwd"]);
  const updated = s["updatedAt"];
  const num = (v: JsonValue | undefined): number | null => (typeof v === "number" ? v : null);
  const roots = s["workspaceRoots"];
  const turns = entry.state?.["turns"];
  const working = ["running", "blocked"].includes(statusType(s));
  return {
    id: entry.id,
    title: sessionTitle(s),
    project: projectOf(cwd),
    cwd,
    badge: badgeOf(entry, stuck, waiting),
    unread: s["isUnseen"] === true,
    updatedAt: typeof updated === "number" ? updated : 0,
    bumpedAt: num(s["bumpedAt"]) ?? (typeof updated === "number" ? updated : 0),
    archived: statusType(s) === "archived" || num(s["archivedAt"]) !== null,
    neverRan: Array.isArray(turns) && turns.length === 0 && !working,
    pinned: num(s["pinnedAt"]) !== null,
    parentId: str(s["parentSessionId"]),
    preview: typeof s["preview"] === "string" ? s["preview"] : "",
    model: str(s["model"]),
    extraFolders: Array.isArray(roots)
      ? roots.filter((r): r is string => typeof r === "string" && r !== cwd)
      : [],
    localOnly: entry.localOnly,
    stuck: entry.id in stuck,
    continuedIn: stuck[entry.id] ?? null,
  };
}
