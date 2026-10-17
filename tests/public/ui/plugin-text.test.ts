/** Plugin label escaping follows native dialog hidden characters and preserves complete visible escapes. */
import { expect, it } from "vitest";
import { displayPluginName } from "../../../ui/src/state/pluginText";

it("exposes the complete native-dialog hidden-character set", () => {
  const points = [
    ...Array.from({ length: 32 }, (_, i) => i),
    ...Array.from({ length: 33 }, (_, i) => 0x7f + i),
    0x061c,
    0x2028,
    0x2029,
    0xfeff,
    ...Array.from({ length: 5 }, (_, i) => 0x202a + i),
    ...Array.from({ length: 4 }, (_, i) => 0x2066 + i),
    ...Array.from({ length: 5 }, (_, i) => 0x200b + i),
  ];
  for (const point of points) {
    expect(displayPluginName(`a${String.fromCodePoint(point)}b`)).toBe(
      `a\\u${point.toString(16).toUpperCase().padStart(4, "0")}b`,
    );
  }
});

it("caps the visible label at 80 characters, without cutting an escape or a Unicode character", () => {
  expect(displayPluginName("Review helper” 🦊")).toBe("Review helper” 🦊");
  expect(displayPluginName("x".repeat(80))).toBe("x".repeat(80));
  expect(displayPluginName("x".repeat(300))).toBe(`${"x".repeat(79)}…`);
  expect(displayPluginName(`${"x".repeat(77)}\u202e`)).toBe(
    `${"x".repeat(77)}…`,
  );
  expect(Array.from(displayPluginName("🦊".repeat(81)))).toHaveLength(80);
});
