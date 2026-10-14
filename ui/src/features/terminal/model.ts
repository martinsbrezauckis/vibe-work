/**
 * The terminal tabs model: which terminals each session has, which one is active, and
 * the last error to show. Pure reducers, no I/O; the store (`store.ts`) feeds them from command
 * results and core `terminal` events.
 * Depends on: protocol/core types. Called by: store.ts, components through the store, tests.
 * Invariants: reducers never mutate; a session's list keeps the core's order (oldest first); the
 * active terminal is always one of the session's terminals, or absent when it has none; closing the
 * active tab activates its right neighbour, else its left one.
 */
import type { TerminalEventView, TerminalView } from "../../protocol/core";

/** Every session's terminal tabs. */
export interface TerminalTabs {
  /** Terminals by session id, oldest first. */
  readonly bySession: Readonly<Record<string, readonly TerminalView[]>>;
  /** The active terminal id per session. */
  readonly active: Readonly<Record<string, string>>;
  /** Sessions whose terminals were read from the core at least once. */
  readonly loaded: Readonly<Record<string, true>>;
  /** The last error per session (shown in the panel until the next success). */
  readonly error: Readonly<Record<string, string>>;
}

/** `record` without `key`. */
function omit<T>(record: Readonly<Record<string, T>>, key: string): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => k !== key));
}

/** No terminals known. */
export const initialTabs: TerminalTabs = { bySession: {}, active: {}, loaded: {}, error: {} };

function withList(
  state: TerminalTabs,
  sessionId: string,
  list: readonly TerminalView[],
): TerminalTabs {
  const active = state.active[sessionId];
  const keep = list.some((t) => t.terminalId === active);
  const newest = list.at(-1);
  const nextActive = keep
    ? state.active
    : newest === undefined
      ? omit(state.active, sessionId)
      : { ...state.active, [sessionId]: newest.terminalId };
  return { ...state, bySession: { ...state.bySession, [sessionId]: list }, active: nextActive };
}

/** Replaces a session's list with the core's (a `list` result). */
export function applyList(
  state: TerminalTabs,
  sessionId: string,
  list: readonly TerminalView[],
): TerminalTabs {
  return { ...withList(state, sessionId, list), loaded: { ...state.loaded, [sessionId]: true } };
}

/** Adds or updates one terminal (opened, exited, restarted). */
export function upsert(state: TerminalTabs, view: TerminalView): TerminalTabs {
  const list = state.bySession[view.sessionId] ?? [];
  const index = list.findIndex((t) => t.terminalId === view.terminalId);
  const next = index < 0 ? [...list, view] : list.map((t, i) => (i === index ? view : t));
  return withList(state, view.sessionId, next);
}

/** Removes a closed terminal; the active tab moves to a neighbour. */
export function remove(state: TerminalTabs, sessionId: string, terminalId: string): TerminalTabs {
  const list = state.bySession[sessionId] ?? [];
  const index = list.findIndex((t) => t.terminalId === terminalId);
  if (index < 0) return state;
  const next = list.filter((t) => t.terminalId !== terminalId);
  if (state.active[sessionId] !== terminalId) return withList(state, sessionId, next);
  const neighbour = next[index] ?? next[index - 1];
  const active =
    neighbour === undefined
      ? omit(state.active, sessionId)
      : { ...state.active, [sessionId]: neighbour.terminalId };
  return { ...state, bySession: { ...state.bySession, [sessionId]: next }, active };
}

/** Makes `terminalId` the session's active tab (ignored when it is not one of its terminals). */
export function setActive(
  state: TerminalTabs,
  sessionId: string,
  terminalId: string,
): TerminalTabs {
  const known = (state.bySession[sessionId] ?? []).some((t) => t.terminalId === terminalId);
  if (!known || state.active[sessionId] === terminalId) return state;
  return { ...state, active: { ...state.active, [sessionId]: terminalId } };
}

/** Sets (or clears, with `null`) the session's error line. */
export function setError(
  state: TerminalTabs,
  sessionId: string,
  message: string | null,
): TerminalTabs {
  if (message === null) {
    if (!(sessionId in state.error)) return state;
    return { ...state, error: omit(state.error, sessionId) };
  }
  return { ...state, error: { ...state.error, [sessionId]: message } };
}

/** Folds one core `terminal` event. */
export function applyTerminalEvent(state: TerminalTabs, event: TerminalEventView): TerminalTabs {
  switch (event.kind) {
    case "opened":
    case "exited":
    case "restarted":
      return upsert(state, event.terminal);
    case "closed":
      return remove(state, event.sessionId, event.terminalId);
  }
}

/** The session's active terminal, if it has one. */
export function activeOf(state: TerminalTabs, sessionId: string): TerminalView | null {
  const id = state.active[sessionId];
  return (state.bySession[sessionId] ?? []).find((t) => t.terminalId === id) ?? null;
}
