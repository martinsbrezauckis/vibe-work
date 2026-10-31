/** The confirmed Chrome work state of this composer’s chat. */
import type { ReactElement } from "react";
import { TEXT } from "../../vocabulary";
import { useCoreState } from "../shell/hooks";

export function ChromeWorkNote({ sessionId }: { sessionId: string }): ReactElement | null {
  const state = useCoreState(
    (s) => s.chromeControl?.work.find((row) => row.sessionId === sessionId)?.state,
  );
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
        : state === "stopping"
          ? TEXT.chromeWorkStopping
          : state === "stopped"
            ? TEXT.chromeWorkStopped
            : state === "disconnected"
              ? TEXT.chromeWorkDisconnected
              : null;
  return label === null ? null : (
    <p className="hint composer-note" role="status" data-testid="composer.chrome-work">
      {label}
    </p>
  );
}
