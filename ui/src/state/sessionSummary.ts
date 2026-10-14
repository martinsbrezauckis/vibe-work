/**
 * What the list column and status bar need to know about one session, derived from the session
 * object Vibe reports (`PublicSession`). Components read these, they never dig into raw JSON.
 * Depends on: protocol/core types. Called by: the list column, the status bar, the UI-control
 * `state` route, the palette, the list model (`features/sessions/listModel.ts`).
 * Invariants: tolerant of the two shapes `status` has been seen in (`{type: "running"}` from Vibe,
 * a bare string from the scripted test server); an unknown status is shown as idle, never thrown.
 */
import type { JsonValue } from "../protocol/core";

/** The badge shown next to a session, including failure state. */
export type Badge = "run" | "input" | "done" | "idle" | "failed" | "stuck";

/** Human text for each badge (also the accessible name). */
export const BADGE_LABEL: Record<Badge, string> = {
  run: "Running",
  input: "Needs input",
  done: "Complete",
  idle: "Idle",
  failed: "Failed",
  stuck: "Vibe can't reopen this session right now",
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
  /** Callbacks (approvals, questions) currently open, from `callbacksChanged`. */
  openCallbackIds: readonly string[];
  /**
   * The latest `RuntimeSnapshot` (models, profiles, active agent) from `runtimeUpdated` or a reply that
   * carried one (the composer's pickers); absent until one arrived.
   */
  runtime?: JsonValue;
}

function str(value: JsonValue | undefined): string | null {
  return typeof value === "string" && value !== "" ? value : null;
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

/** The folder name used to group sessions into projects. */
export function projectOf(cwd: string | null): string {
  if (cwd === null) return "No folder";
  const parts = cwd.split(/[\\/]+/).filter((p) => p !== "");
  return parts[parts.length - 1] ?? "No folder";
}

/** The badge for a session entry (`stuck` wins: nothing can run until the user acts). */
export function badgeOf(entry: SessionEntry, stuck: StuckMap = {}): Badge {
  if (entry.id in stuck) return "stuck";
  const type = statusType(entry.session);
  // An open callback only counts while the session is working: Vibe 2.26.0 does not always tell us
  // that an answered callback was resolved (live, after a `session/resume`: the turn completed and the
  // callback entry never got its update), and an idle session cannot be waiting for anything.
  const working = type !== "idle" && type !== "failed" && type !== "archived";
  if (type === "blocked" || (working && entry.openCallbackIds.length > 0)) return "input";
  if (type === "running") return "run";
  if (type === "failed") return "failed";
  return entry.session["isUnseen"] === true ? "done" : "idle";
}

/** Derives the row for one entry. */
export function summarize(entry: SessionEntry, stuck: StuckMap = {}): SessionSummary {
  const s = entry.session;
  const cwd = str(s["cwd"]);
  const updated = s["updatedAt"];
  const num = (v: JsonValue | undefined): number | null => (typeof v === "number" ? v : null);
  const roots = s["workspaceRoots"];
  return {
    id: entry.id,
    title: str(s["title"]) ?? str(s["preview"]) ?? "Untitled session",
    project: projectOf(cwd),
    cwd,
    badge: badgeOf(entry, stuck),
    unread: s["isUnseen"] === true,
    updatedAt: typeof updated === "number" ? updated : 0,
    bumpedAt: num(s["bumpedAt"]) ?? (typeof updated === "number" ? updated : 0),
    archived: statusType(s) === "archived" || num(s["archivedAt"]) !== null,
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
