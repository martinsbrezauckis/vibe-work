/**
 * The banner of a session whose review baseline was lost or tampered with. Changes are shown read only
 * against the last commit. A new baseline is taken only when the user clicks "Start reviewing from now". The core never does that on its own.
 * Depends on: store.ts. Called by: ChangesTab.
 */
import { Button } from "../../components/Controls";
import type { ReactElement } from "react";

import type { ChangesView } from "../../protocol/core";
import type { Changes } from "./store";

/** The lost-baseline banner. */
export function LostBanner(props: {
  changes: Changes;
  view: ChangesView;
  busy: boolean;
}): ReactElement {
  const { changes, view, busy } = props;
  return (
    <div className="key-problem" role="alert">
      <p>
        The starting point for this session was lost ({view.reason ?? "unknown reason"}). Showing
        changes against the last commit instead. Nothing here can be kept or reverted until you
        start reviewing from now.
      </p>
      <Button
        data-testid="changes.lost-banner-restart"
        type="button"
        appearance="compact"
        disabled={busy}
        data-action="restart"
        onClick={() => void changes.restart(view.sessionId)}
      >
        Start reviewing from now
      </Button>
    </div>
  );
}
