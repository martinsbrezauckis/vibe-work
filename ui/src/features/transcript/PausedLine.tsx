/**
 * The chat's line while the person has paused the agent's browser: it says in words that the agent's browser and
 * App preview actions wait for the person (Pause holds only those tools; the agent's other work goes on), with Resume
 * and Stop. Its words are said once per change by the app's polite
 * announcer; the line itself is no live region (a region whose role and text arrive together is not read by every
 * screen reader, and a second region would say it twice). The words are a paragraph, which every web view exposes
 * (WebKitGTK drops the text of a plain span laid out as a flex item).
 * Depends on: usePausedTools, the composer's Stop, the announcer. Called by: Transcript (above the conversation,
 * beside the todos).
 */
import { type ReactElement, useEffect } from "react";

import { Button } from "../../components/Controls";
import { announce } from "../../state/announcer";
import { usePausedTools } from "../../state/usePausedTools";
import { useStore } from "../../state/createStore";
import { TEXT, wording } from "../../vocabulary";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { runningTurnId } from "../composer/model/session";

/** What the line says: waiting (a browser call is held) or paused (nothing held yet). Words: the vocabulary file. */
export function pausedLineText(waiting: boolean, place: string): string {
  return waiting ? TEXT.pausedBrowserWaiting : wording(TEXT.pausedBrowserLine, { place });
}

/** Renders the line for `sessionId`, or nothing while its browser is not paused. */
export function PausedLine({ sessionId }: { sessionId: string }): ReactElement | null {
  const app = useApp();
  const paused = usePausedTools(app.core, sessionId);
  const entry = useCoreState((s) => s.sessions[sessionId]);
  const turnId = runningTurnId(entry?.state ?? null, entry?.session);
  const stopRequested = useStore(app.composer.store, (s) => s.stopRequested[sessionId]);
  const stopping = stopRequested !== undefined && (turnId === null || stopRequested === turnId);
  const browser = stopping ? null : paused.browser;
  const text = browser === null ? null : pausedLineText(browser.waiting, browser.place);
  useEffect(() => {
    if (text !== null) announce(text);
  }, [text]);
  return (
    <div
      className="paused-line"
      data-testid="transcript.paused-line"
      data-state={browser === null ? "none" : browser.waiting ? "waiting" : "paused"}
    >
      {browser !== null && (
        <>
          <span className="paused-line-dot" aria-hidden="true" />
          <p className="paused-line-text">{text}</p>
          <Button
            data-testid="transcript.paused-line-resume"
            type="button"
            title="Resume browser access"
            aria-label="Resume browser access"
            onClick={() => void paused.resume()}
          >
            Resume
          </Button>
          <Button
            data-testid="transcript.paused-line-stop"
            type="button"
            title="Stop the agent"
            aria-label="Stop the agent"
            onClick={() => void app.composer.actions?.stop(sessionId)}
          >
            Stop
          </Button>
          {paused.error !== null && <span className="paused-line-error">{paused.error}</span>}
        </>
      )}
    </div>
  );
}
