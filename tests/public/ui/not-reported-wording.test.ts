/** Missing value labels agree across model context, account counts and session costs; valid zero stays a value. */
import { expect, it } from "vitest";
import { contextWindowLabel } from "../../../ui/src/features/composer/model/contextWindow";
import { formatTokens } from "../../../ui/src/state/account";
import { costText } from "../../../ui/src/state/sessionUsage";

it("uses one complete missing-value label for absent and invalid context, counts and costs", () => {
  expect(formatTokens(null)).toBe("Not reported");
  expect(costText(null)).toBe("Not reported");
  for (const source of [undefined, "catalog"]) {
    for (const maxContextLength of [
      undefined,
      null,
      0,
      -1,
      1.5,
      Infinity,
      NaN,
    ]) {
      expect(contextWindowLabel({ source, maxContextLength })).toBe(
        "Context: Not reported",
      );
    }
  }
  expect(formatTokens(0)).toBe("0");
  expect(costText(0)).toBe("$0.00");
  expect(
    contextWindowLabel({ source: "catalog", maxContextLength: 32_768 }),
  ).toBe("Context: 32,768 tokens");
});
