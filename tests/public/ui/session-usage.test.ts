/** Cost estimates follow Vibe's cache pricing and preserve unknown readings; the native ledger owns child costs. */
import { expect, it } from "vitest";
import {
  sessionUsageOf,
  subagentUsageOf,
  costText,
} from "../../../ui/src/state/sessionUsage";

it("uses reported cache rates, falls back to input rates and clamps cache overcounts", () => {
  const stats = {
    sessionPromptTokens: 1000000,
    sessionCompletionTokens: 100000,
    sessionCachedTokens: 800000,
    inputPricePerMillion: 2,
    outputPricePerMillion: 6,
    cachedInputPricePerMillion: 0.2,
  };
  expect(sessionUsageOf({ stats }, undefined).cost).toBeCloseTo(1.16);
  expect(
    sessionUsageOf(
      { stats: { ...stats, cachedInputPricePerMillion: null } },
      undefined,
    ).cost,
  ).toBeCloseTo(2.6);
  expect(
    sessionUsageOf(
      { stats: { ...stats, sessionCachedTokens: 2000000 } },
      undefined,
    ).cost,
  ).toBeCloseTo(0.8);
  expect(sessionUsageOf({}, undefined).cost).toBeNull();
  expect(costText(0.000004)).toBe("$0.000004");
});

it("reads resumed-session runtime stats and native child prices without adding them to the parent", () => {
  expect(
    sessionUsageOf({}, { stats: { steps: 4, sessionCachedTokens: 42 } }),
  ).toMatchObject({ steps: 4, cached: 42, cost: null });
  expect(
    subagentUsageOf({
      subagents: [
        {
          sessionId: "child",
          name: "Research",
          costUsd: 0.12,
          steps: 2,
          pricingFallback: true,
          usage: { cachedInputTokens: 90 },
        },
      ],
    }),
  ).toEqual([
    {
      id: "child",
      name: "Research",
      cost: 0.12,
      steps: 2,
      cached: 90,
      estimated: true,
    },
  ]);
});
