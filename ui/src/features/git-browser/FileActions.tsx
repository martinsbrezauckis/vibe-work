/** Changed-file row actions. Depends on the admission reason; called by GitBrowser using existing action classes. */
import { Button } from "../../components/Controls";
import { useId, type ReactElement } from "react";
import type { BrowserTarget } from "./model";
import { filePointReason } from "./point";

export function FileActions({
  target,
  path,
  onPoint,
}: {
  target: BrowserTarget;
  path: string;
  onPoint: (() => void) | undefined;
}): ReactElement {
  const reason = filePointReason(target);
  const reasonId = useId();
  return (
    <div className="gb-actions" role="group" aria-label={`Actions for ${path}`}>
      <Button
        data-testid="git-browser.point-file"
        type="button"
        appearance="compact"
        disabled={reason !== null}
        title={reason ?? "Point Vibe at this"}
        aria-describedby={reason === null ? undefined : reasonId}
        onClick={onPoint}
      >
        Point Vibe at this
      </Button>
      {reason !== null && (
        <span className="gb-reason" id={reasonId}>
          {reason}
        </span>
      )}
    </div>
  );
}
