/** Monthly allowance projection and refresh store. Depends on the typed core client and createStore.
 * Settings follows one session/key pair; stale replies and provider text never enter the view.
 * Vibe exposes percentages, reset time and PAYG enabled, not monetary amounts or a PAYG balance.
 */
import type { JsonValue } from "../protocol/core";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";

export type PlanUsage =
  | { kind: "loading" }
  | { kind: "notReported" }
  | { kind: "unavailable" }
  | { kind: "error"; message: string }
  | {
      kind: "ready";
      used: number;
      left: number;
      payg: boolean | null;
      resetAt: string | null;
      observedAt: number;
    };

function object(value: JsonValue | undefined) {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value : null;
}

const NOT_REPORTED = "Mistral did not report plan usage. Refresh or check your plan online.";

/** Only a known non-plan key is unavailable; missing optional billing is a read failure. */
export function parsePlanUsage(result: JsonValue, now = Date.now()): PlanUsage {
  if (object(result)?.["detached"] === true) return { kind: "notReported" };
  const account = object(object(result)?.["account"]);
  const status = account?.["status"];
  if (status === "unauthorized")
    return {
      kind: "error",
      message: "Mistral did not accept the key. Check your key in Account, then refresh.",
    };
  if (status === "missing_key")
    return {
      kind: "error",
      message: "No Mistral key is set up. Add a key in Account, then refresh.",
    };
  if (status !== "ready")
    return {
      kind: "error",
      message: "Vibe could not read your plan. Refresh or check your connection.",
    };
  const kind = object(account?.["plan"])?.["kind"];
  if (kind === "api" || kind === "mistral_code") return { kind: "unavailable" };
  const budget = object(account?.["budget"]);
  const used = budget?.["usagePercentage"];
  if (kind !== "chat" || typeof used !== "number" || !Number.isFinite(used) || used < 0)
    return { kind: "error", message: NOT_REPORTED };
  const resetAt = budget?.["resetAt"];
  return {
    kind: "ready",
    used,
    left: Math.max(0, 100 - used),
    payg: typeof budget?.["paygEnabled"] === "boolean" ? budget["paygEnabled"] : null,
    resetAt:
      typeof resetAt === "string" &&
      /^\d{4}-\d{2}-\d{2}T/.test(resetAt) &&
      Number.isFinite(Date.parse(resetAt))
        ? resetAt
        : null,
    observedAt: now,
  };
}

/** Percent of the included allowance, independent of currency and session token counts. */
export function planPercent(value: number): string {
  return `${value.toLocaleString("en-US", { maximumFractionDigits: 1 })}%`;
}

interface Reading {
  sessionId: string;
  itemId: string | null;
  value: PlanUsage;
}

/** Follows the visible key/session only; a refresh clears old numbers before requesting new ones. */
export function createPlanUsage() {
  const store = createStore<Reading | null>(null);
  let generation = 0;
  let current: { core: CoreController; sessionId: string; itemId: string | null } | null = null;
  const refresh = async () => {
    const target = current;
    if (target === null) return;
    const request = ++generation;
    store.set({ sessionId: target.sessionId, itemId: target.itemId, value: { kind: "loading" } });
    let value: PlanUsage;
    try {
      const reply = await target.core.run({
        type: "call",
        method: "account/read",
        params: { sessionId: target.sessionId },
      });
      value = parsePlanUsage(reply.type === "reply" ? reply.result : null);
    } catch {
      value = {
        kind: "error",
        message: "Vibe could not read your plan. Refresh or check your connection.",
      };
    }
    if (request === generation)
      store.set({ sessionId: target.sessionId, itemId: target.itemId, value });
  };
  return {
    store,
    refresh,
    follow(core: CoreController, sessionId: string, itemId: string | null) {
      current = { core, sessionId, itemId };
      void refresh();
      return () => {
        ++generation;
        current = null;
        store.set(null);
      };
    },
  };
}
