/** Turn and step durations round once, so a value just under a unit boundary carries ("4m00s", never "3m60s"). */
import { expect, it } from "vitest";
import { formatElapsed } from "../../../ui/src/features/transcript/rows/Time";

it("keeps the existing formats", () => {
  expect(formatElapsed(850)).toBe("850ms");
  expect(formatElapsed(3200)).toBe("3.2s");
  expect(formatElapsed(42_000)).toBe("42s");
  expect(formatElapsed(125_000)).toBe("2m05s");
  expect(formatElapsed(183_000)).toBe("3m03s");
});

it("carries a rounded-up second into the next minute", () => {
  expect(formatElapsed(239_600)).toBe("4m00s");
  expect(formatElapsed(239_500)).toBe("4m00s");
  expect(formatElapsed(239_499)).toBe("3m59s");
  expect(formatElapsed(119_700)).toBe("2m00s");
});

it("carries at the smaller unit boundaries too", () => {
  expect(formatElapsed(59_600)).toBe("1m00s");
  expect(formatElapsed(999.6)).toBe("1.0s");
  expect(formatElapsed(9_960)).toBe("10s");
});
