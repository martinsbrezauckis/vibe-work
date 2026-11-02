/** Derives session spending and counters from Vibe stats; Settings renders these without a local price list. */
import type { JsonValue } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";

type Obj = Record<string, JsonValue | undefined>;
function object(value: JsonValue | undefined): Obj {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : {};
}
function number(value: JsonValue | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
}

/** Reported counters and a USD estimate at Vibe's reported rates, discounting cached input as Vibe does. */
export function sessionUsageOf(
  session: Obj | undefined,
  runtime: JsonValue | undefined,
): {
  cost: number | null;
  steps: number | null;
  cached: number | null;
  tools: number | null;
} {
  const stats = object(session?.["stats"] ?? object(runtime)["stats"]);
  const input = number(stats["sessionPromptTokens"]);
  const output = number(stats["sessionCompletionTokens"]);
  const cached = number(stats["sessionCachedTokens"]);
  const inputPrice = number(stats["inputPricePerMillion"]);
  const outputPrice = number(stats["outputPricePerMillion"]);
  const cachedPrice = number(stats["cachedInputPricePerMillion"]) ?? inputPrice;
  const cache = Math.min(cached ?? 0, input ?? 0);
  const cost =
    input === null ||
    output === null ||
    inputPrice === null ||
    outputPrice === null ||
    cachedPrice === null
      ? null
      : ((input - cache) * inputPrice + cache * cachedPrice + output * outputPrice) / 1_000_000;
  const succeeded = number(stats["toolCallsSucceeded"]);
  const failed = number(stats["toolCallsFailed"]);
  const rejected = number(stats["toolCallsRejected"]);
  return {
    cost,
    steps: number(stats["steps"]),
    cached,
    tools:
      succeeded === null && failed === null && rejected === null
        ? null
        : (succeeded ?? 0) + (failed ?? 0) + (rejected ?? 0),
  };
}

/** Dollar amounts preserve small nonzero costs; null means Vibe has not reported pricing. */
export function costText(cost: number | null): string {
  return cost === null
    ? "Not reported yet"
    : cost.toLocaleString("en-US", {
        style: "currency",
        currency: "USD",
        minimumFractionDigits: 2,
        maximumFractionDigits: cost > 0 && cost < 0.01 ? 6 : 2,
      });
}

/** Safe subset of the native subagent ledger (identifiers stay off screen). */
export interface SubagentUsage {
  id: string;
  name: string;
  cost: number | null;
  estimated: boolean;
  steps: number | null;
  cached: number | null;
}

/** Native ledger rows with unavailable fields kept unknown. */
export function subagentUsageOf(value: JsonValue): SubagentUsage[] {
  const rows = object(value)["subagents"];
  return !Array.isArray(rows)
    ? []
    : rows.flatMap((value) => {
        const row = object(value);
        if (typeof row["sessionId"] !== "string" || typeof row["name"] !== "string") return [];
        return [
          {
            id: row["sessionId"],
            name: row["name"],
            cost: number(row["costUsd"]),
            estimated: row["pricingFallback"] === true,
            steps: number(row["steps"]),
            cached: number(object(row["usage"])["cachedInputTokens"]),
          },
        ];
      });
}

/** Owns ledger reads while Usage is open; late replies and polling stop when the caller leaves. */
export function createSubagentUsage() {
  const store = createStore<{
    sessionId: string;
    rows: SubagentUsage[];
    problem: string | null;
  } | null>(null);
  return {
    store,
    follow(core: CoreController, sessionId: string): () => void {
      let live = true;
      let timer: ReturnType<typeof setTimeout>;
      const read = async () => {
        try {
          const reply = await core.run({
            type: "call",
            method: "session/subagents/read",
            params: { sessionId, timeoutMs: 1000 },
          });
          if (live && reply.type === "reply")
            store.set({ sessionId, rows: subagentUsageOf(reply.result), problem: null });
        } catch (error) {
          if (live) store.set({ sessionId, rows: [], problem: toCallError(error).error.message });
        }
        if (live)
          timer = setTimeout(() => {
            void read();
          }, 5000);
      };
      void read();
      return () => {
        live = false;
        clearTimeout(timer);
      };
    },
  };
}
