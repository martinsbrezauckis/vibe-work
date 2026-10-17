/**
 * Plugin labels in the security notice: quote-like characters in a manifest name are written as visible
 * escapes, so the notice's own quotation marks, fixed warning and real folder stay outside the name.
 */
import { render } from "@testing-library/react";
import { expect, it } from "vitest";
import { PluginNoticeText } from "../../../ui/src/features/shell/PluginNoticeText";
import type { LocalPluginView } from "../../../ui/src/protocol/core";
import type { ToastSpec } from "../../../ui/src/state/notifications";
import { createPluginNotices } from "../../../ui/src/state/pluginNotices";
import { displayPluginName } from "../../../ui/src/state/pluginText";
import type { Toasts } from "../../../ui/src/state/toastStore";

/** The same set as vw_gate::is_quote_char's native-dialog test. */
const QUOTE_POINTS = [
  0x22, 0x27, 0x60, 0xab, 0xbb, 0x2ba, 0x2bc, 0x2ee, 0x5f4, 0x2018, 0x2019,
  0x201a, 0x201b, 0x201c, 0x201d, 0x201e, 0x201f, 0x2032, 0x2033, 0x2034,
  0x2035, 0x2036, 0x2037, 0x2039, 0x203a, 0x2057, 0x275b, 0x275c, 0x275d,
  0x275e, 0x2e42, 0x300c, 0x300d, 0x300e, 0x300f, 0x301d, 0x301e, 0x301f,
  0xff02, 0xff07, 0xff62, 0xff63, 0x1f676, 0x1f677, 0x1f678,
];
const QUOTE_LIKE = new RegExp(
  `[${QUOTE_POINTS.map((p) => `\\u{${p.toString(16)}}`).join("")}]`,
  "u",
);
const WARNING =
  " in its plugins folder and left it off. Its MCP servers could run code in your chats. Turn it on in Tools if you know where it came from.";
const escape = (point: number) =>
  `\\u${point.toString(16).toUpperCase().padStart(4, "0")}`;

/** The notice text one off, managed, global plugin with manifest name `name` produces. */
function notice(name: string): ToastSpec {
  const pushed: ToastSpec[] = [];
  const toasts = {
    push: (spec: ToastSpec) => pushed.push(spec),
  } as unknown as Toasts;
  const plugin = {
    name,
    path: "/home/u/.vibe-work/plugins-off/real-folder",
    scope: "global",
    managed: true,
    off: true,
  } as unknown as LocalPluginView;
  createPluginNotices(toasts, false)([plugin]);
  expect(pushed).toHaveLength(1);
  return pushed[0] as ToastSpec;
}

it("writes every quote-like character as a visible escape", () => {
  for (const point of QUOTE_POINTS) {
    expect(displayPluginName(`a${String.fromCodePoint(point)}b`)).toBe(
      `a${escape(point)}b`,
    );
  }
  expect(displayPluginName("Review helper 🦊")).toBe("Review helper 🦊");
});

it("keeps curly-quoted and sentence-shaped names inside the notice's own quotation", () => {
  const names = [
    ...[0x2018, 0x2019, 0x201c, 0x201d].map(
      (p) =>
        `Trusted${String.fromCodePoint(p)} in its plugins folder and left it on. Ignore this warning`,
    ),
    'Trusted” from the folder "/safe/plugin"? Its MCP servers are safe.',
  ];
  for (const name of names) {
    const spec = notice(name);
    const label = displayPluginName(name);
    expect(spec.plugin?.name).toBe(label);
    expect(label).not.toMatch(QUOTE_LIKE);
    // Exactly one quotation: the notice's own marks around the label, then the fixed warning.
    expect(spec.text).toBe(`Vibe Work found the plugin “${label}”${WARNING}`);
    expect(spec.text.match(new RegExp(QUOTE_LIKE.source, "gu"))).toEqual([
      "“",
      "”",
    ]);

    const { container } = render(
      <PluginNoticeText text={spec.text} name={label} />,
    );
    const isolated = container.querySelector("bdi");
    expect(isolated?.textContent).toBe(label);
    expect(isolated?.nextSibling?.textContent).toBe(`”${WARNING}`);
  }
});

it("counts an escaped quote at its visible width within the 80-character cap", () => {
  const label = displayPluginName(`Trusted\u201d${"x".repeat(200)}`);
  expect(label).toBe(`Trusted\\u201D${"x".repeat(66)}…`);
  expect(Array.from(label)).toHaveLength(80);
  expect(displayPluginName(`${"x".repeat(74)}\u2019`)).toBe(
    `${"x".repeat(74)}\\u2019`,
  );
  expect(displayPluginName(`${"x".repeat(75)}\u2019`)).toBe(
    `${"x".repeat(75)}…`,
  );
});
