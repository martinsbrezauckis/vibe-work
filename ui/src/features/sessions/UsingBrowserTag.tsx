/**
 * The session row's dot while the chat's agent controls a browser through the helper (Chrome, Edge, Firefox,
 * Safari): a small dot in the chat's frame colour, named "Using Chrome" (or the browser it uses), so the person sees
 * in the sidebar which chats are driving a browser. The same rule as the helper frame's Working state: the chat's
 * browser session is active or connecting, a turn of the chat runs, and the browser is not paused (the Paused mark
 * shows then instead).
 * Depends on: the core state (`chromeControl.work`, `chromeControl.frameColours`, the chat's turns, `pausedBrowser`),
 * the frame palette. Called by: sessions/SessionRow.
 * Invariants: render only; never a control (no click, no focus).
 */
import type { ReactElement } from "react";

import type { ChromeWorkView } from "../../protocol/core";
import { runningTurnId } from "../composer/model/session";
import { frameColour } from "../preview/AgentFrame";
import { useCoreState } from "../shell/hooks";

/** The dot's words for the chat, or null when its agent is not controlling a helper browser now. */
export function usingBrowserText(
  work: readonly ChromeWorkView[] | undefined,
  sessionId: string,
  turnRunning: boolean,
  paused: boolean,
): string | null {
  if (!turnRunning || paused) return null;
  const row = work?.find(
    (entry) =>
      entry.sessionId === sessionId && (entry.state === "active" || entry.state === "connecting"),
  );
  return row === undefined ? null : `Using ${row.browser ?? "Chrome"}`;
}

/** The dot itself: an image with a name, coloured with the chat's frame colour. */
export function UsingBrowserMark({
  words,
  colour,
}: {
  words: string;
  colour: number | undefined;
}): ReactElement {
  return (
    <span
      className="using-browser-row"
      role="img"
      aria-label={words}
      title={words}
      data-testid="sessions.using-browser"
    >
      <span
        className="using-browser-dot"
        aria-hidden="true"
        style={{ "--agent-frame": frameColour(colour) } as React.CSSProperties}
      />
    </span>
  );
}

/** The dot for `sessionId`'s row, or nothing. */
export function UsingBrowserTag({ sessionId }: { sessionId: string }): ReactElement | null {
  const work = useCoreState((s) => s.chromeControl?.work);
  const colour = useCoreState((s) => s.chromeControl?.frameColours?.[sessionId]);
  const turnRunning = useCoreState((s) => {
    const entry = s.sessions[sessionId];
    return runningTurnId(entry?.state ?? null, entry?.session) !== null;
  });
  const paused = useCoreState((s) => s.pausedBrowser[sessionId] !== undefined);
  const words = usingBrowserText(work, sessionId, turnRunning, paused);
  return words === null ? null : <UsingBrowserMark words={words} colour={colour} />;
}
