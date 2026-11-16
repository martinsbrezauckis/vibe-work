/**
 * Inline browser activity indicator while the session's agent acts (or acted within the last 30 s), with Stop;
 * after Stop it offers Resume while a turn of the chat runs (with no turn running the Stop holds until the person's next
 * message, which lifts it, so nothing is shown). The user sees when the agent drives the page and can take it back.
 * Its words are said once per change by the app's polite announcer (as the agent frame's line); the strip itself is no
 * live region, because a screen reader re-read a role=status strip on every re-render (NVDA: 3-4 times per change).
 * Depends on: protocol types, the announcer. Called by: PreviewTab.
 */
import { useEffect, type ReactElement } from "react";

import { Button } from "../../components/Controls";
import { announce } from "../../state/announcer";

import type { BrowserState } from "../../protocol/core";

/** The strip, or nothing when the agent is not using the browser and nothing is stopped. */
export function AgentStrip({
  state,
  turnRunning,
  onStop,
  onResume,
}: {
  state: BrowserState | undefined;
  /** True while a turn of the chat runs. */
  turnRunning: boolean;
  onStop: () => void;
  onResume: () => void;
}): ReactElement | null {
  const words = stripWords(state, turnRunning);
  useEffect(() => {
    if (words !== null) announce(words);
  }, [words]);
  if (state === undefined || words === null) return null;
  if (state.stopped) {
    return (
      <div className="browser-strip" data-state="stopped" title={words}>
        <span className="browser-strip-dot" aria-hidden="true" />
        <span className="browser-strip-text">{words}</span>
        <Button
          data-testid="preview.agent-strip-preview-btn"
          type="button"
          appearance="preview"
          title="Resume"
          onClick={onResume}
        >
          Resume
        </Button>
      </div>
    );
  }
  return (
    <div className="browser-strip" data-state={state.inFlight ? "acting" : "active"} title={words}>
      <span className="browser-strip-dot" aria-hidden="true" />
      <span className="browser-strip-text">{words}</span>
      <Button
        data-testid="preview.agent-strip-preview-btn-2"
        type="button"
        appearance="preview"
        placement="browser-stop"
        title="Stop"
        onClick={onStop}
      >
        Stop
      </Button>
    </div>
  );
}

/** The strip's words, or null when it shows nothing. */
function stripWords(state: BrowserState | undefined, turnRunning: boolean): string | null {
  if (
    state === undefined ||
    (!state.agentActive && !state.inFlight && !state.stopped && !state.pendingNativeClick)
  )
    return null;
  if (state.stopped)
    return turnRunning
      ? "You stopped the agent's use of the browser. It may use it again after your next message."
      : null;
  return (
    state.pendingNativeClick ??
    (state.inFlight ? "Agent is using the browser" : "Agent used the browser just now")
  );
}
