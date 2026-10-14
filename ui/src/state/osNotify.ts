/**
 * Which session changes deserve an OS notification, and the rules that decide whether one is shown.
 * Pure: `transitions` compares two core models, `shouldNotify` applies the user's level and the
 * focus rule, `textFor` words it. `notifyStore.ts` runs them and calls the host.
 * Depends on: coreModel types, sessionSummary. Called by: notifyStore, tests.
 * Invariants:
 * - Levels nest: `errors` < `permissions` (errors + permission requests) < `all` (+ finished).
 *   The default is `permissions`.
 * - A notification is shown only when the window is not focused, or the session is not the one open
 *   in the window (the user is looking at it otherwise).
 * - The OS body has a short event summary and bounded session name; the full title and any
 *   structured failure message stay in the app's notification history.
 * - A session seen for the first time produces no transition (opening the app must not announce
 *   every old session), and a child (subagent) session never announces "finished".
 */
import type { CoreState } from "./coreModel";
import { statusType } from "./sessionSummary";

/** What the user chose to hear about. */
export type NotifyLevel = "errors" | "permissions" | "all";

/** The levels in order, for the settings control. */
export const NOTIFY_LEVELS: readonly NotifyLevel[] = ["errors", "permissions", "all"];

/** The default level (errors and permission requests). */
export const DEFAULT_NOTIFY_LEVEL: NotifyLevel = "permissions";

/** Why a notification is considered. */
export type NotifyKind = "error" | "permission" | "finished";

/** One session change worth a notification. */
export interface Transition {
  sessionId: string;
  kind: NotifyKind;
  title: string;
  /** The exact permission card, when the transition opened one. */
  callbackId?: string;
  /** Vibe's full failed-status message, when present. */
  detail?: string;
}

const BODY_LIMIT = 120;

function object(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** Notifications `prev` -> `next` calls for (one per newly opened callback). */
export function transitions(prev: CoreState, next: CoreState): Transition[] {
  if (prev.sessions === next.sessions) return [];
  const found: Transition[] = [];
  for (const [id, entry] of Object.entries(next.sessions)) {
    const before = prev.sessions[id];
    if (before === undefined) continue;
    const was = statusType(before.session);
    const now = statusType(entry.session);
    const title =
      typeof entry.session["title"] === "string" && entry.session["title"] !== ""
        ? entry.session["title"]
        : `Session ${id}`;
    const opened = entry.openCallbackIds.filter((c) => !before.openCallbackIds.includes(c));
    const isChild = typeof entry.session["parentSessionId"] === "string";
    const kinds: NotifyKind[] = [];
    if (now === "failed" && was !== "failed") kinds.push("error");
    if (opened.length > 0) kinds.push("permission");
    // A blocked status with no callback id cannot identify a card. The callback list is the
    // source of truth for permission notifications; status can oscillate while a card is open.
    if (kinds.length === 0 && was === "running" && now === "idle" && !isChild)
      kinds.push("finished");
    for (const kind of kinds) {
      const message = object(entry.session["status"])?.["message"];
      for (const callbackId of kind === "permission" ? opened : [undefined]) {
        found.push({
          sessionId: id,
          kind,
          title,
          ...(callbackId === undefined ? {} : { callbackId }),
          ...(kind === "error" && typeof message === "string" && message.trim() !== ""
            ? { detail: message }
            : {}),
        });
      }
    }
  }
  return found;
}

/** Does `level` include `kind`? */
export function levelAllows(level: NotifyLevel, kind: NotifyKind): boolean {
  if (kind === "error") return true;
  if (kind === "permission") return level !== "errors";
  return level === "all";
}

/** Where the user is looking right now. */
export interface Attention {
  /** The app window has focus. */
  focused: boolean;
  /** The session open in the window (`null` for the new-session view). */
  openSessionId: string | null;
}

/** The full rule: level allows it, and the user is not already looking at that session. */
export function shouldNotify(level: NotifyLevel, t: Transition, where: Attention): boolean {
  if (!levelAllows(level, t.kind)) return false;
  return !where.focused || where.openSessionId !== t.sessionId;
}

const SUMMARY: Record<NotifyKind, string> = {
  error: "Stopped with an error",
  permission: "Needs your answer",
  finished: "Finished",
};

/** Short, readable OS title and body. The session name is bounded for system displays. */
export function textFor(t: Transition): { title: string; body: string } {
  const prefix = `${SUMMARY[t.kind]} · `;
  const remaining = BODY_LIMIT - prefix.length;
  const name = t.title.trim() || `Session ${t.sessionId}`;
  const bounded = name.length > remaining ? `${name.slice(0, remaining - 1)}…` : name;
  return { title: "Vibe Work", body: `${prefix}${bounded}` };
}

/** Full text retained in the app and shown after notification activation. */
export function fullTextFor(t: Transition): string {
  const summary: Record<NotifyKind, string> = {
    error: "stopped with an error",
    permission: "needs your answer to continue",
    finished: "finished",
  };
  const detail = t.kind === "error" && t.detail !== undefined ? `: ${t.detail}` : ".";
  return `${t.title} ${summary[t.kind]}${detail}`;
}
