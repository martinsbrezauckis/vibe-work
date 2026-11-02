/**
 * A running turn's model-request retries (the proxy or Mistral unreachable, server errors, rate limits) as the
 * run bar shows them: the attempt and the reason in plain words. Vibe announces each attempt with a
 * `turn/retrying` notice (no state change on the wire) and carries the current one in a snapshot's
 * `retrying`; the turn moving on (a new or changed history entry, a turn update) ends it.
 * Depends on: sessionSummary's entry type. Called by: coreModel (folding), the composer (label).
 */
import type { JsonValue } from "../protocol/core";
import type { SessionEntry } from "./sessionSummary";

/** The retry a session's running turn is in. */
export interface TurnRetry {
  /** Vibe's `PublicRetryCategory` (unknown values are kept and read as no reason). */
  category: string;
  /** 1 for the first retry. */
  attempt: number;
}

const REASONS: Readonly<Record<string, string>> = {
  connection: "No connection",
  rate_limited: "Rate limited",
  server_error: "Server error",
  timed_out: "Timed out",
};

function object(value: JsonValue | undefined): Record<string, JsonValue | undefined> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
}

/** The retry a snapshot's state carries, else `undefined`. */
export function retryOfState(
  state: Record<string, JsonValue | undefined> | null,
): TurnRetry | undefined {
  const retrying = object(state?.["retrying"]);
  if (retrying === null) return undefined;
  const attempt = retrying["retryAttempt"];
  return {
    category: typeof retrying["category"] === "string" ? retrying["category"] : "unknown",
    attempt: typeof attempt === "number" && attempt > 0 ? attempt : 1,
  };
}

/** `entry` after one `turn/retrying` notice: the next attempt, with its reason. */
export function withRetryNotice(entry: SessionEntry, params: JsonValue): SessionEntry {
  const category = object(params)?.["category"];
  return {
    ...entry,
    retry: {
      category: typeof category === "string" ? category : "unknown",
      attempt: (entry.retry?.attempt ?? 0) + 1,
    },
  };
}

/** `entry` without a retry (the turn moved on). */
export function withoutRetry(entry: SessionEntry): SessionEntry {
  if (entry.retry === undefined) return entry;
  const rest = { ...entry };
  delete rest.retry;
  return rest;
}

/** "Retrying (2) · Server error"; the reason is left out when Vibe gave none. */
export function retryLabel(retry: TurnRetry): string {
  const reason = REASONS[retry.category];
  const head = `Retrying (${String(retry.attempt)})…`;
  return reason === undefined ? head : `${head} · ${reason}`;
}
