/** The app's two announcement regions per system: on Linux each is a read-only text field outside the tab order
 * whose text is the message (the Linux web view reports a text field's text change to screen readers, not a
 * paragraph's); elsewhere each is a paragraph. Both keep the same text, live politeness and atomic reading.
 *
 * PREDICT: the regions were paragraphs on every system, so the Linux expectations (a read-only field, outside the
 * tab order, whose value is the message) fail.
 */
import { act, render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import {
  Announcer,
  regionsAreTextFields,
} from "../../../ui/src/features/shell/Announcer";
import { announce } from "../../../ui/src/state/announcer";

const LINUX =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)";
const MACOS =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)";
const WINDOWS =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36 Edg/157.0.0.0";

describe("announcement regions", () => {
  it("are text fields only in the Linux web view", () => {
    expect(regionsAreTextFields(LINUX)).toBe(true);
    expect(regionsAreTextFields(MACOS)).toBe(false);
    expect(regionsAreTextFields(WINDOWS)).toBe(false);
  });

  it("on Linux hold the message as a read-only field's text, outside the tab order", () => {
    const { container } = render(<Announcer userAgent={LINUX} />);
    act(() => {
      announce("Vibe finished.");
      announce("Copy failed.", { assertive: true });
    });
    const polite = container.querySelector<HTMLTextAreaElement>(
      '[data-announcer="polite"]',
    );
    const assertive = container.querySelector<HTMLTextAreaElement>(
      '[data-announcer="assertive"]',
    );
    for (const [region, live, text] of [
      [polite, "polite", "Vibe finished."],
      [assertive, "assertive", "Copy failed."],
    ] as const) {
      expect(region?.tagName).toBe("TEXTAREA");
      expect(region?.readOnly).toBe(true);
      expect(region?.tabIndex).toBe(-1);
      expect(region?.getAttribute("aria-live")).toBe(live);
      expect(region?.getAttribute("aria-atomic")).toBe("true");
      expect(region?.hasAttribute("aria-label")).toBe(false);
      expect(region?.value).toBe(text);
      expect(region?.textContent).toBe(text);
    }
  });

  it("elsewhere are paragraphs with the message as their text", () => {
    const { container } = render(<Announcer userAgent={MACOS} />);
    act(() => {
      announce("Vibe finished.");
    });
    const polite = container.querySelector('[data-announcer="polite"]');
    expect(polite?.tagName).toBe("P");
    expect(polite?.getAttribute("aria-live")).toBe("polite");
    expect(polite?.textContent).toBe("Vibe finished.");
  });
});
