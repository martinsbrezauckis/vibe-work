/**
 * Inline browser activity indicator while the session's agent acts (or acted within the last 30 s). It has no controls
 * of its own: the person pauses the agent with Pause/Resume in the frame around the page and stops it with the chat's
 * Stop, so after a Stop the strip shows nothing.
 * Its words are said once per change by the app's polite announcer (as the agent frame's line); the strip itself is no
 * live region, because a screen reader re-read a role=status strip on every re-render (NVDA: 3-4 times per change).
 * Depends on: protocol types, the announcer. Called by: PreviewTab.
 */
import { useEffect, type ReactElement } from "react";

import { announce } from "../../state/announcer";

import type { BrowserState } from "../../protocol/core";

/** The strip, or nothing when the agent is not using the browser or is stopped. */
export function AgentStrip({ state }: { state: BrowserState | undefined }): ReactElement | null {
  const words = stripWords(state);
  useEffect(() => {
    if (words !== null) announce(words);
  }, [words]);
  if (state === undefined || words === null) return null;
  return (
    <div className="browser-strip" data-state={state.inFlight ? "acting" : "active"} title={words}>
      <span className="browser-strip-dot" aria-hidden="true" />
      <span className="browser-strip-text">{words}</span>
    </div>
  );
}

/** The strip's words, or null when it shows nothing. */
function stripWords(state: BrowserState | undefined): string | null {
  if (
    state === undefined ||
    state.stopped ||
    (!state.agentActive && !state.inFlight && !state.pendingNativeClick)
  )
    return null;
  return (
    state.pendingNativeClick ??
    (state.inFlight ? "Agent is using the browser" : "Agent used the browser just now")
  );
}
