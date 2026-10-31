/**
 * Which session changes deserve an OS notification, and the rules that decide whether one is shown.
 * Pure: `transitions` compares two core models, `shouldNotify` applies the user's level and the
 * focus rule, `textFor` words it. `notifyStore.ts` runs them and calls the host.
 * Depends on: coreModel's live card projection, sessionSummary. Called by: notifyStore, tests.
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
import { promptHost } from "../features/ask/waiting";
import { cardAsks, toolTransport } from "./askBell";
import { pendingHistoryAsks } from "../features/transcript/model/protectedChange";
import { type CoreState, subagentCardsOf } from "./coreModel";
import { sessionTitle, statusType } from "./sessionSummary";

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
  /** The subject of a native prompt, separate from its chat title. */
  subject?: string;
  /** Vibe's full failed-status message, when present. */
  detail?: string;
}

const BODY_LIMIT = 120;

function object(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** The callbacks a parent chat hosts as subagent cards, by callback id -> the hosting parent. */
export function hostedCards(state: CoreState): Map<string, string> {
  const hosted = new Map<string, string>();
  for (const [id, entry] of Object.entries(state.sessions))
    for (const card of subagentCardsOf(state.subagentCards, id, entry.state?.["childSessions"]))
      hosted.set(card.callbackId, id);
  return hosted;
}

/** The announcement of a subagent card that parent chat `hostId` hosts (the card is in the parent's view). */
export function hostedCardTransition(
  state: CoreState,
  hostId: string,
  callbackId: string,
): Transition {
  return {
    sessionId: hostId,
    kind: "permission",
    title: `Subagent in ${sessionTitle(state.sessions[hostId]?.session)}`,
    callbackId,
  };
}

/**
 * Notifications `prev` -> `next` calls for (one per newly opened callback). A child chat's callback that its
 * parent hosts as a subagent card is announced once, by the parent, even when the child chat is loaded too. One the
 * loaded child announced before the card arrived is not announced again; the notifier moves that entry to the
 * parent (`notifyStore`), whose view has the card.
 */
export function transitions(prev: CoreState, next: CoreState): Transition[] {
  if (
    prev.sessions === next.sessions &&
    prev.subagentCards === next.subagentCards &&
    prev.asks === next.asks
  )
    return [];
  const found: Transition[] = [];
  const hosted = hostedCards(next);
  const hostedBefore = hostedCards(prev);
  for (const ask of Object.values(next.asks)) {
    if (
      ask.surface === "native" &&
      ask.requester !== "core" &&
      (ask.requester !== "debugApi" || ask.personOnly) &&
      prev.asks[ask.id] === undefined
    ) {
      const sessionId = promptHost(next, ask) ?? "";
      found.push({
        sessionId,
        kind: "permission",
        title: sessionTitle(next.sessions[sessionId]?.session),
        subject: ask.title,
        callbackId: ask.id,
      });
    }
  }
  for (const [id, entry] of Object.entries(next.sessions)) {
    const before = prev.sessions[id];
    if (before === undefined) continue;
    const was = statusType(before.session);
    const now = statusType(entry.session);
    const title = sessionTitle(entry.session);
    const earlier = [
      ...before.openCallbackIds,
      ...cardAsks(prev, id),
      ...pendingHistoryAsks(before.state?.["history"]),
    ];
    const hostedElsewhere = (c: string): boolean => (hosted.get(c) ?? id) !== id;
    const opened = [
      ...entry.openCallbackIds,
      ...cardAsks(next, id),
      ...pendingHistoryAsks(entry.state?.["history"]),
    ].filter(
      (c, i, all) =>
        all.indexOf(c) === i &&
        !earlier.includes(c) &&
        !toolTransport(next, id, c) &&
        !hostedElsewhere(c),
    );
    const earlierChildren = new Set(
      subagentCardsOf(prev.subagentCards, id, before.state?.["childSessions"]).map(
        (card) => card.callbackId,
      ),
    );
    for (const card of subagentCardsOf(next.subagentCards, id, entry.state?.["childSessions"])) {
      if (earlierChildren.has(card.callbackId) || opened.includes(card.callbackId)) continue;
      // Already announced by the loaded child chat before the parent's card arrived.
      if (!hostedBefore.has(card.callbackId) && openIn(prev, card.sessionId, card.callbackId))
        continue;
      // The parent hosts this card; the child has no independently loaded transcript to open.
      found.push(hostedCardTransition(next, id, card.callbackId));
    }
    const isChild = typeof entry.session["parentSessionId"] === "string";
    const kinds: NotifyKind[] = [];
    if (now === "failed" && was !== "failed") kinds.push("error");
    if (opened.length > 0) kinds.push("permission");
    // A blocked status with no callback id cannot identify a card. The callback list is the
    // source of truth for permission notifications; status can oscillate while a card is open.
    // A chat with a waiting prompt has not finished (a held approval can read idle); its finish is
    // announced when the last prompt closes.
    const waitsNow = waiting(next, id, entry.openCallbackIds, entry.state?.["history"]);
    const waitedBefore = waiting(prev, id, before.openCallbackIds, before.state?.["history"]);
    if (
      kinds.length === 0 &&
      now === "idle" &&
      !isChild &&
      !waitsNow &&
      (was === "running" || waitedBefore)
    )
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

/** Whether callback `c` was open in session `id` in `state` (its own list, card asks or a pending history ask). */
function openIn(state: CoreState, id: string, c: string): boolean {
  const entry = state.sessions[id];
  return (
    entry !== undefined &&
    (entry.openCallbackIds.includes(c) ||
      cardAsks(state, id).includes(c) ||
      pendingHistoryAsks(entry.state?.["history"]).includes(c))
  );
}

/** Whether a prompt of session `id` waits in `state`: a card, a pending history ask, or a window of its own. */
function waiting(
  state: CoreState,
  id: string,
  callbacks: readonly string[],
  history: Parameters<typeof pendingHistoryAsks>[0],
): boolean {
  return (
    callbacks.length > 0 ||
    cardAsks(state, id).length > 0 ||
    pendingHistoryAsks(history).length > 0 ||
    Object.values(state.asks).some(
      (ask) => ask.session === id && ask.surface === "native" && ask.requester === "agent",
    )
  );
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
  const subject =
    t.subject === undefined
      ? undefined
      : t.subject.length > 60
        ? `${t.subject.slice(0, 59)}…`
        : t.subject;
  const prefix = `${subject === undefined ? SUMMARY[t.kind] : `Needs your answer: ${subject}`} · `;
  const remaining = BODY_LIMIT - prefix.length;
  const name = sessionTitle({ title: t.title });
  const bounded = name.length > remaining ? `${name.slice(0, remaining - 1)}…` : name;
  return { title: "Vibe Work", body: `${prefix}${bounded}` };
}

/** Full text retained in the app and shown after notification activation. */
export function fullTextFor(t: Transition): string {
  if (t.kind === "permission" && t.subject !== undefined)
    return `Vibe needs your answer about ${t.subject} in “${t.title}”.`;
  const summary: Record<NotifyKind, string> = {
    error: "stopped with an error",
    permission: "needs your answer to continue",
    finished: "finished",
  };
  const detail = t.kind === "error" && t.detail !== undefined ? `: ${t.detail}` : ".";
  return `${sessionTitle({ title: t.title })} ${summary[t.kind]}${detail}`;
}
