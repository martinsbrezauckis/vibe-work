/**
 * Inline browser activity indicator while the session's agent acts (or acted within the last 30 s), with Stop;
 * after Stop it offers Resume. State text is exposed through the status name and tooltip. The user sees when the
 * agent drives the page and can take it back.
 * Depends on: protocol types. Called by: PreviewTab.
 */
import type { ReactElement } from "react";

import type { BrowserState } from "../../protocol/core";

/** The strip, or nothing when the agent is not using the browser and nothing is stopped. */
export function AgentStrip({
  state,
  onStop,
  onResume,
}: {
  state: BrowserState | undefined;
  onStop: () => void;
  onResume: () => void;
}): ReactElement | null {
  if (
    state === undefined ||
    (!state.agentActive && !state.inFlight && !state.stopped && !state.pendingNativeClick)
  )
    return null;
  if (state.stopped) {
    return (
      <div
        className="browser-strip"
        data-state="stopped"
        role="status"
        aria-label="You stopped the agent's use of the browser. It may use it again after your next message."
        title="You stopped the agent's use of the browser. It may use it again after your next message."
      >
        <span className="browser-strip-dot" aria-hidden="true" />
        <span className="browser-strip-text">
          You stopped the agent's use of the browser. It may use it again after your next message.
        </span>
        <button
          data-testid="preview.agent-strip-preview-btn"
          type="button"
          className="preview-btn"
          title="Resume"
          onClick={onResume}
        >
          Resume
        </button>
      </div>
    );
  }
  const message =
    state.pendingNativeClick ??
    (state.inFlight ? "Agent is using the browser" : "Agent used the browser just now");
  return (
    <div
      className="browser-strip"
      data-state={state.inFlight ? "acting" : "active"}
      role="status"
      aria-label={message}
      title={message}
    >
      <span className="browser-strip-dot" aria-hidden="true" />
      <span className="browser-strip-text">{message}</span>
      <button
        data-testid="preview.agent-strip-preview-btn-2"
        type="button"
        className="preview-btn browser-stop"
        title="Stop"
        onClick={onStop}
      >
        Stop
      </button>
    </div>
  );
}
