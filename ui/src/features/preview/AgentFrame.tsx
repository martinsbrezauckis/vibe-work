/**
 * The built-in browser's agent control frame: while the agent works in this chat's browser, a coloured frame
 * around the page and a top-centred line with the chat name, the status, Pause and Stop; while the person paused
 * it, a grey dashed frame with Resume and Stop. Drawn by the app outside the page, so the page cannot read or
 * change it. Its words are said once per change by the app's polite announcer; the line itself is no live region
 * (a second region would say them twice). The words are a paragraph, which every web view exposes to screen readers
 * (WebKitGTK drops the text of a plain span laid out as a flex item).
 * Depends on: usePausedTools, the composer's Stop, the announcer. Called by: PreviewTab (above the page area).
 */
import { type ReactElement, useEffect } from "react";

import { Button } from "../../components/Controls";
import { announce } from "../../state/announcer";
import type { BrowserState } from "../../protocol/core";

/** The frame palette by a chat's colour index (the same eight as the helper browsers' frames); every chat starts at
 * index 0, and a chat keeps another index once its colour is set (kept across restarts by the core). */
export const FRAME_COLOURS = [
  "var(--agent-frame-0)",
  "var(--agent-frame-1)",
  "var(--agent-frame-2)",
  "var(--agent-frame-3)",
  "var(--agent-frame-4)",
  "var(--agent-frame-5)",
  "var(--agent-frame-6)",
  "var(--agent-frame-7)",
] as const;

/** The CSS colour of a chat's frame from its stored index (unknown indexes show index 0). */
export function frameColour(colour: number | undefined): string {
  return FRAME_COLOURS[colour ?? 0] ?? FRAME_COLOURS[0];
}

/** What the frame shows. */
export type FrameState = "working" | "paused" | "off";

/** The frame's state from the agent's browser activity, the running turn and the person's pause. */
export function frameState(
  agent: BrowserState | undefined,
  turnRunning: boolean,
  paused: boolean,
): FrameState {
  if (!turnRunning) return "off";
  if (paused) return "paused";
  if (agent !== undefined && !agent.stopped && (agent.inFlight || agent.agentActive))
    return "working";
  return "off";
}

/** The status words on the frame's line. */
export function frameText(state: FrameState, chat: string): string {
  return state === "paused"
    ? `Paused — you are using the browser · ${chat}`
    : `Vibe Work · ${chat} · Working`;
}

/** Renders the line (the frame itself is the page area's outline, styled by `data-agent-frame`). */
export function AgentFrame({
  state,
  chat,
  colour = 0,
  onPause,
  onResume,
  onStop,
}: {
  state: FrameState;
  /** The chat name (at most 40 characters are shown). */
  chat: string;
  /** The chat's frame colour index. */
  colour?: number;
  onPause: () => void;
  onResume: () => void;
  onStop: () => void;
}): ReactElement {
  const name = chat.length > 40 ? `${chat.slice(0, 39)}…` : chat;
  const accent = frameColour(colour);
  const words = state === "off" ? null : frameText(state, name);
  useEffect(() => {
    if (words !== null) announce(words);
  }, [words]);
  return (
    <div
      className="agent-frame-line"
      data-testid="preview.agent-frame"
      data-state={state}
      style={{ "--agent-frame": accent } as React.CSSProperties}
    >
      {words !== null && (
        <>
          <span className="agent-frame-dot" aria-hidden="true" />
          <p className="agent-frame-text">{words}</p>
          {state === "working" ? (
            <Button
              data-testid="preview.agent-frame-pause"
              type="button"
              appearance="preview"
              title="Pause browser access"
              aria-label="Pause browser access"
              onClick={onPause}
            >
              Pause
            </Button>
          ) : (
            <Button
              data-testid="preview.agent-frame-resume"
              type="button"
              appearance="preview"
              title="Resume browser access"
              aria-label="Resume browser access"
              onClick={onResume}
            >
              Resume
            </Button>
          )}
          <Button
            data-testid="preview.agent-frame-stop"
            type="button"
            appearance="preview"
            title="Stop the agent"
            aria-label="Stop the agent"
            onClick={onStop}
          >
            Stop
          </Button>
        </>
      )}
    </div>
  );
}
