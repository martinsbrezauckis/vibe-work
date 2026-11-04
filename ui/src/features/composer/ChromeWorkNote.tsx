/** The confirmed browser work state of this composer’s chat (Your Chrome, Your Edge, …). */
import type { ReactElement } from "react";
import { TEXT } from "../../vocabulary";
import { arr, obj } from "../transcript/model/entries";
import { useCoreState } from "../shell/hooks";

export function ChromeWorkNote({ sessionId }: { sessionId: string }): ReactElement | null {
  const state = useCoreState(
    (s) => s.chromeControl?.work.find((row) => row.sessionId === sessionId)?.state,
  );
  // The row names the browser of the chat's session (Chrome, Edge, …); the texts are written for Chrome.
  const browser = useCoreState(
    (s) => s.chromeControl?.work.find((row) => row.sessionId === sessionId)?.browser,
  );
  // Work state includes normal cleanup. Only an interrupted/failed turn confirms a stop to report.
  const interrupted = useCoreState((s) => {
    const turn = obj(arr(s.sessions[sessionId]?.state?.["turns"]).at(-1));
    return turn?.["status"] === "interrupted" || turn?.["status"] === "failed";
  });
  const error = useCoreState((s) => s.chromeControl?.selectionError);
  if (error)
    return (
      <p className="hint composer-note" role="alert" data-testid="composer.chrome-selection-error">
        {error}
      </p>
    );
  const label =
    state === "active"
      ? TEXT.chromeWorkActive
      : state === "connecting"
        ? TEXT.chromeWorkConnecting
        : state === "stopping" && interrupted
          ? TEXT.chromeWorkStopping
          : state === "stopped" && interrupted
            ? TEXT.chromeWorkStopped
            : state === "disconnected"
              ? TEXT.chromeWorkDisconnected
              : null;
  return label === null ? null : (
    <p className="hint composer-note" role="status" data-testid="composer.chrome-work">
      {browser && browser !== "Chrome" ? label.replace("Chrome", browser) : label}
    </p>
  );
}
