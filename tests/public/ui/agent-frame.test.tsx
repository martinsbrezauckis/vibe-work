/** The built-in browser's agent control frame and the chat's paused line: the frame shows working with Pause and
 * Stop, paused with Resume and Stop, nothing when the agent is not working; buttons have names; the line says that
 * browser actions wait (not the whole agent); the chat's row is marked while paused. Renders the actual components
 * with generated core types.
 *
 * PREDICT (written before the components): the imports fail (no frame or paused line exists), so every test fails.
 */
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { BrowserState } from "../../../ui/src/protocol/core";
import {
  AgentFrame,
  FRAME_COLOURS,
  frameColour,
  frameState,
  frameText,
} from "../../../ui/src/features/preview/AgentFrame";
import { pausedLineText } from "../../../ui/src/features/transcript/PausedLine";
import { pausedTagText } from "../../../ui/src/features/sessions/PausedTag";
import { applyEvent, initialCore } from "../../../ui/src/state/coreModel";
import { TEXT, wording } from "../../../ui/src/vocabulary";

const agent = (over: Partial<BrowserState>): BrowserState => ({
  sessionId: "chat",
  agentActive: false,
  inFlight: false,
  stopped: false,
  lastAgentAtMs: null,
  log: [],
  receipts: [],
  ...over,
});

describe("frame state", () => {
  it("is working only while the turn runs and the agent uses the browser", () => {
    expect(frameState(agent({ inFlight: true }), true, false)).toBe("working");
    expect(frameState(agent({ agentActive: true }), true, false)).toBe(
      "working",
    );
    expect(frameState(agent({ inFlight: true }), false, false)).toBe("off");
    expect(frameState(agent({}), true, false)).toBe("off");
    expect(
      frameState(agent({ inFlight: true, stopped: true }), true, false),
    ).toBe("off");
    expect(frameState(undefined, true, true)).toBe("paused");
    expect(frameState(agent({ inFlight: true }), false, true)).toBe("off");
  });
});

describe("agent frame", () => {
  it("shows the chat name, Working, Pause and Stop while the agent works", () => {
    const onPause = vi.fn();
    const onStop = vi.fn();
    render(
      <AgentFrame
        state="working"
        chat="Order <b>shoes</b>"
        onPause={onPause}
        onResume={vi.fn()}
        onStop={onStop}
      />,
    );
    // The words are said by the app's announcer; the line itself is no live region.
    const line = screen.getByTestId("preview.agent-frame");
    expect(line).not.toHaveAttribute("aria-live");
    expect(line).not.toHaveAttribute("role");
    expect(line).toHaveTextContent("Vibe Work · Order <b>shoes</b> · Working");
    fireEvent.click(
      screen.getByRole("button", { name: "Pause browser access" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Stop the agent" }));
    expect(onPause).toHaveBeenCalledOnce();
    expect(onStop).toHaveBeenCalledOnce();
    expect(
      screen.queryByRole("button", { name: "Resume browser access" }),
    ).toBeNull();
  });

  it("offers Resume and Stop while paused", () => {
    const onResume = vi.fn();
    const onStop = vi.fn();
    render(
      <AgentFrame
        state="paused"
        chat="Order shoes"
        onPause={vi.fn()}
        onResume={onResume}
        onStop={onStop}
      />,
    );
    expect(screen.getByTestId("preview.agent-frame")).toHaveTextContent(
      frameText("paused", "Order shoes"),
    );
    expect(frameText("paused", "Order shoes")).toBe(
      "Paused: you are using the browser · Order shoes",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Resume browser access" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Stop the agent" }));
    expect(onResume).toHaveBeenCalledOnce();
    expect(onStop).toHaveBeenCalledOnce();
    expect(
      screen.queryByRole("button", { name: "Pause browser access" }),
    ).toBeNull();
  });

  it("shows no buttons and no words when the agent is not working", () => {
    render(
      <AgentFrame
        state="off"
        chat="x"
        onPause={vi.fn()}
        onResume={vi.fn()}
        onStop={vi.fn()}
      />,
    );
    expect(screen.queryAllByRole("button")).toHaveLength(0);
    expect(screen.getByTestId("preview.agent-frame")).toHaveTextContent("");
    expect(screen.getByTestId("preview.agent-frame")).toHaveAttribute(
      "data-state",
      "off",
    );
  });

  it("shortens a long chat name to 40 characters", () => {
    const long = "a".repeat(80);
    render(
      <AgentFrame
        state="working"
        chat={long}
        onPause={vi.fn()}
        onResume={vi.fn()}
        onStop={vi.fn()}
      />,
    );
    expect(screen.getByTestId("preview.agent-frame")).toHaveTextContent(
      `Vibe Work · ${"a".repeat(39)}… · Working`,
    );
  });
});

describe("paused line", () => {
  // Pause holds only the browser and App preview tools; the agent's other tools keep running. The line says so and
  // never claims that the agent or "nothing" runs.
  it("says that browser actions wait, not the whole agent", () => {
    expect(pausedLineText(false, "Chrome")).toBe(
      "You paused the browser (Chrome). The agent's browser and App preview actions wait until you resume; its other work goes on.",
    );
    expect(pausedLineText(true, "Chrome")).toBe(
      "The agent's browser action is waiting for you. It runs when you resume, or ends when you press Stop.",
    );
    for (const waiting of [true, false]) {
      expect(pausedLineText(waiting, "Chrome")).not.toMatch(
        /Nothing runs|The agent waits|agent is waiting\./,
      );
    }
  });

  it("takes its words from the vocabulary file", () => {
    expect(pausedLineText(false, "Chrome")).toBe(
      wording(TEXT.pausedBrowserLine, { place: "Chrome" }),
    );
    expect(pausedLineText(true, "Chrome")).toBe(TEXT.pausedBrowserWaiting);
  });
});

describe("sidebar mark", () => {
  it("follows the chat's pause events and forgets a resumed chat", () => {
    const paused = applyEvent(initialCore, {
      type: "pausedTools",
      view: {
        sessionId: "chat",
        paused: [
          { tool: "browser", place: "Chrome", sinceMs: 1, waiting: true },
        ],
      },
    });
    expect(paused.pausedBrowser).toEqual({ chat: true });
    const resumed = applyEvent(paused, {
      type: "pausedTools",
      view: { sessionId: "chat", paused: [] },
    });
    expect(resumed.pausedBrowser).toEqual({});
  });

  it("says whether the agent waits", () => {
    expect(pausedTagText(true)).toBe(
      "Browser paused: the agent is waiting for you",
    );
    expect(pausedTagText(false)).toBe("Browser paused");
  });
});

// PREDICT (written before the per-chat colour): one palette entry only, so a chat's stored index 3 shows index 0.
describe("frame colour", () => {
  it("shows the chat's stored palette index and falls back to the default", () => {
    expect(FRAME_COLOURS).toHaveLength(8);
    expect(frameColour(3)).toBe("var(--agent-frame-3)");
    expect(frameColour(undefined)).toBe("var(--agent-frame-0)");
    expect(frameColour(9)).toBe("var(--agent-frame-0)");
  });

  it("draws the line in the chat's colour", () => {
    render(
      <AgentFrame
        state="working"
        chat="Coloured chat"
        colour={5}
        onPause={vi.fn()}
        onResume={vi.fn()}
        onStop={vi.fn()}
      />,
    );
    expect(
      screen
        .getByTestId("preview.agent-frame")
        .style.getPropertyValue("--agent-frame"),
    ).toBe("var(--agent-frame-5)");
  });
});
