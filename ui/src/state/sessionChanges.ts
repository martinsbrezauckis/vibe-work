/**
 * Applies the core's typed session changes (`SessionChange`) to the UI's copy of a session.
 * The Rust store stays the state of record; this is the same reduction on the screen side so that
 * components read one store. Mirrors `crates/vw-session/src/reducer.rs`.
 * Depends on: jsonPatch, sessionSummary types. Called by: coreModel.
 * Invariants: pure (returns a new entry, never mutates); a change for a session the UI does not
 * hold is ignored (a snapshot comes first by the subscription's rule); a patch that cannot be
 * applied leaves the entry as it was and reports `false` so the caller can ask for a re-read.
 */
import type { JsonValue, SessionChange } from "../protocol/core";
import { applyPatch } from "./jsonPatch";
import { currentModeId } from "../features/composer/model/access";
import type { SessionEntry } from "./sessionSummary";

type Obj = Record<string, JsonValue | undefined>;

function isObj(value: JsonValue | undefined): value is Obj {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function upsertById(
  list: JsonValue | undefined,
  item: JsonValue,
  keys: readonly string[],
): JsonValue[] {
  const items = Array.isArray(list) ? [...list] : [];
  const idOf = (v: JsonValue | undefined): JsonValue | undefined =>
    isObj(v) ? keys.map((k) => v[k]).find((x) => x !== undefined) : undefined;
  const id = idOf(item);
  const at = id === undefined ? -1 : items.findIndex((x) => idOf(x) === id);
  if (at >= 0) items[at] = item;
  else items.push(item);
  return items;
}

/** Result of applying one change. */
export interface Applied {
  entry: SessionEntry;
  /** False when a patch failed and the entry is unchanged. */
  ok: boolean;
}

function withState(entry: SessionEntry, edit: (state: Obj) => Obj): SessionEntry {
  if (entry.state === null) return entry;
  const state = edit({ ...entry.state });
  const session = isObj(state["session"]) ? state["session"] : entry.session;
  return { ...entry, state, session };
}

/** Applies `change` to `entry` (the entry the change's session id names). */
export function applyChange(entry: SessionEntry, change: SessionChange): Applied {
  try {
    return { entry: reduce(entry, change), ok: true };
  } catch {
    return { entry, ok: false };
  }
}

function reduce(entry: SessionEntry, change: SessionChange): SessionEntry {
  switch (change.kind) {
    case "sessionPatched": {
      const patched = applyPatch(entry.session, change.patch);
      if (!isObj(patched)) throw new Error("session patch produced a non-object");
      const next = { ...entry, session: patched };
      return entry.state === null ? next : { ...next, state: { ...entry.state, session: patched } };
    }
    case "entryUpserted":
      return withState(entry, (s) => ({
        ...s,
        history: upsertById(s["history"], change.entry, ["id"]),
      }));
    case "entryPatched":
      return withState(entry, (s) => {
        const history = Array.isArray(s["history"]) ? [...s["history"]] : [];
        const at = history.findIndex((e) => isObj(e) && e["id"] === change.entryId);
        const target = history[at];
        if (at < 0 || target === undefined) throw new Error("entry not found");
        history[at] = applyPatch(target, change.patch);
        return { ...s, history };
      });
    case "turnUpserted":
      return withState(entry, (s) => ({
        ...s,
        turns: upsertById(s["turns"], change.turn, ["id"]),
      }));
    case "queueReplaced":
      return withState(entry, (s) => ({ ...s, turnQueue: change.queue }));
    case "tokenUsageSet":
      return withState(entry, (s) => ({
        ...s,
        session: {
          ...(isObj(s["session"]) ? s["session"] : {}),
          tokenUsage: change.tokenUsage,
          // Context fill: only when the stats event carried it. Not `contextUsage`: that name
          // is Vibe's own session field (the token usage of the current context).
          ...(change.contextUsage === null ? {} : { contextFill: change.contextUsage }),
        },
      }));
    case "childUpserted":
      return withState(entry, (s) => ({
        ...s,
        childSessions: upsertById(s["childSessions"], change.child, ["id", "sessionId"]),
      }));
    case "callbacksChanged":
      return { ...entry, openCallbackIds: change.openCallbackIds };
    case "runtimeUpdated": {
      const accessMode = change.accessMode ?? currentModeId(change.runtime, undefined);
      return { ...entry, runtime: change.runtime, ...(accessMode === null ? {} : { accessMode }) };
    }
    case "notice":
    case "removed":
    case "handedOff":
      return entry;
  }
}
