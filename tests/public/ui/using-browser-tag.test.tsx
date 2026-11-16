/** The sidebar's "Using Chrome" dot: shown for a chat whose agent controls a helper browser in a running turn, named
 * after that browser, coloured with the chat's frame colour, hidden while paused or with no running turn.
 *
 * PREDICT (written before the dot): the module does not exist, so the import fails and every test fails.
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { ChromeWorkView } from "../../../ui/src/protocol/core";
import {
  UsingBrowserMark,
  usingBrowserText,
} from "../../../ui/src/features/sessions/UsingBrowserTag";

const work: ChromeWorkView[] = [
  { sessionId: "chat", state: "active", browser: "Edge" },
  { sessionId: "other", state: "stopped" },
];

describe("using-browser dot", () => {
  it("names the browser the chat's agent controls in a running turn", () => {
    expect(usingBrowserText(work, "chat", true, false)).toBe("Using Edge");
    expect(
      usingBrowserText(
        [{ sessionId: "chat", state: "connecting" }],
        "chat",
        true,
        false,
      ),
    ).toBe("Using Chrome");
  });

  it("is absent without a running turn, while paused, or for a stopped session", () => {
    expect(usingBrowserText(work, "chat", false, false)).toBeNull();
    expect(usingBrowserText(work, "chat", true, true)).toBeNull();
    expect(usingBrowserText(work, "other", true, false)).toBeNull();
    expect(usingBrowserText(undefined, "chat", true, false)).toBeNull();
  });

  it("is a named image in the chat's frame colour, never a control", () => {
    render(<UsingBrowserMark words="Using Chrome" colour={2} />);
    const mark = screen.getByRole("img", { name: "Using Chrome" });
    expect(mark.tagName).toBe("SPAN");
    expect(mark.querySelector("button, a, [tabindex]")).toBeNull();
    const dot = mark.querySelector<HTMLElement>(".using-browser-dot");
    expect(dot?.style.getPropertyValue("--agent-frame")).toBe(
      "var(--agent-frame-2)",
    );
  });
});
