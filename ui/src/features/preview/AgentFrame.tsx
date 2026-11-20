/**
 * The built-in browser's agent control frame: while the agent works in this chat's browser, a coloured frame
 * around the page and a top-centred line with the chat name, the status, Pause and Stop; while the person paused
 * it, a grey dashed frame with Resume and Stop. Drawn by the app outside the page, so the page cannot read or
 * change it. Its words are said once per change by the app's polite announcer; the line itself is no live region
 * (a second region would say them twice). Pause and Resume are two buttons, not one renamed button: activating the
 * focused one hands focus to the other when the state changes, so a screen reader hears one new focus (a focused
 * button renamed in place was spoken twice); focus the person moved elsewhere meanwhile stays there. The words are a paragraph, which every web view exposes to screen readers
 * (WebKitGTK drops the text of a plain span laid out as a flex item).
 * Depends on: usePausedTools, the composer's Stop, the announcer. Called by: PreviewTab (above the page area).
 */
import { type ReactElement, useEffect, useRef, useState } from "react";

import { Button } from "../../components/Controls";
import { announce, cancelAnnouncement } from "../../state/announcer";
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

/**
 * The frame's state for PreviewTab ([`frameState`]), with one rule for a pause's end: when the pause ends while no
 * agent action is in flight, the frame shows nothing until the agent acts again. Stop ends the pause before the
 * agent's Stop reaches the browser state, so without this the frame passed through "Working" (and said it) on its way
 * to nothing; a Resume that lets a waiting action go on is in flight and shows Working at once.
 */
export function useFrameState(
  agent: BrowserState | undefined,
  turnRunning: boolean,
  paused: boolean,
  stopRequested = false,
  sessionId?: string | null,
): FrameState {
  const raw = frameState(agent, turnRunning, paused);
  const inFlight = agent?.inFlight === true;
  const key = `${sessionId ?? ""}|${raw}|${String(inFlight)}`;
  const [seen, setSeen] = useState({ key, raw, held: false, sessionId });
  let current = seen;
  if (seen.key !== key) {
    // Derived from the previous render's state (React's "store information from previous renders" pattern).
    const held =
      seen.sessionId === sessionId &&
      raw === "working" &&
      !inFlight &&
      (seen.raw === "paused" || seen.held);
    current = { key, raw, held, sessionId };
    setSeen(current);
  }
  return stopRequested || current.held ? "off" : raw;
}

/** The status words on the frame's line. */
export function frameText(state: FrameState, chat: string): string {
  return state === "paused"
    ? `Paused: you are using the browser · ${chat}`
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
  reserve = false,
}: {
  state: FrameState;
  /** The chat name (at most 40 characters are shown). */
  chat: string;
  /** The chat's frame colour index. */
  colour?: number;
  onPause: () => void;
  onResume: () => void;
  onStop: () => void;
  /** Keep the line's space while it is off (PreviewTab: always, while the panel shows the chat), so the native page
   * below never moves: not at a turn's start or end, not at the agent's first browser action, not between actions. */
  reserve?: boolean;
}): ReactElement {
  const name = chat.length > 40 ? `${chat.slice(0, 39)}…` : chat;
  const accent = frameColour(colour);
  const words = state === "off" ? null : frameText(state, name);
  useEffect(() => {
    if (state !== "working") cancelAnnouncement(frameText("working", name));
    if (words !== null) announce(words);
  }, [name, state, words]);
  // Set when the focused Pause or Resume was activated; the other button takes focus once it replaces it.
  const handOver = useRef(false);
  const toggle = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!handOver.current || toggle.current === null) return;
    const lost = document.activeElement === null || document.activeElement === document.body;
    if (lost) toggle.current.focus();
    handOver.current = false;
  }, [state]);
  const act = (run: () => void) => (event: React.MouseEvent<HTMLButtonElement>) => {
    handOver.current = document.activeElement === event.currentTarget;
    run();
  };
  return (
    <div
      className="agent-frame-line"
      data-testid="preview.agent-frame"
      data-state={state}
      data-reserved={reserve && words === null ? "" : undefined}
      style={{ "--agent-frame": accent } as React.CSSProperties}
    >
      {words !== null && (
        <>
          <span className="agent-frame-dot" aria-hidden="true" />
          <p className="agent-frame-text">{words}</p>
          {state === "working" ? (
            <Button
              key="pause"
              ref={toggle}
              data-testid="preview.agent-frame-pause"
              type="button"
              shape="outlined"
              placement="preview"
              title="Pause browser access"
              aria-label="Pause browser access"
              onClick={act(onPause)}
            >
              Pause
            </Button>
          ) : (
            <Button
              key="resume"
              ref={toggle}
              data-testid="preview.agent-frame-resume"
              type="button"
              shape="outlined"
              placement="preview"
              title="Resume browser access"
              aria-label="Resume browser access"
              onClick={act(onResume)}
            >
              Resume
            </Button>
          )}
          <Button
            data-testid="preview.agent-frame-stop"
            type="button"
            shape="outlined"
            placement="preview"
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
