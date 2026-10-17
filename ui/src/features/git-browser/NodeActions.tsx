/** Primary next step and remaining existing actions. Depends on browser model; called by GitBrowser rows. */
import { useId, type ReactElement } from "react";
import { actionStep, nextStep, type BrowserAction, type BrowserTarget } from "./model";

export function NodeActions({
  target,
  onAction,
  selectedSessionId,
}: {
  target: BrowserTarget;
  selectedSessionId?: string | null;
  onAction: (target: BrowserTarget, action: BrowserAction) => void;
}): ReactElement {
  const step = nextStep(target);
  const reasonId = useId();
  const actions: BrowserAction[] = ["commit", "push", "pr", "bring", "new-chat", "new-task"];
  if (target.kind === "copy" && target.restricted) {
    if (target.sessionId && target.sessionId === selectedSessionId)
      return (
        <p className="gb-state">
          This is the copy this chat works in. Use the Changes tab to save, publish or bring it into
          main.
        </p>
      );
    return (
      <div className="gb-actions" role="group" aria-label={`Actions for ${target.name}`}>
        <button
          data-testid="git-browser.primary-action"
          type="button"
          className="mini"
          data-action={`git-browser-${step.action}`}
          onClick={() => {
            onAction(target, step.action);
          }}
        >
          {step.label}
        </button>
      </div>
    );
  }
  return (
    <div className="gb-actions" role="group" aria-label={`Actions for ${target.name}`}>
      <button
        data-testid="git-browser.primary-action"
        type="button"
        className="mini"
        data-action={`git-browser-${step.action}`}
        disabled={step.reason !== null}
        title={step.reason ?? step.label}
        aria-describedby={step.reason === null ? undefined : reasonId}
        onClick={() => {
          onAction(target, step.action);
        }}
      >
        {step.label}
      </button>
      <details className="gb-more">
        <summary
          data-testid="git-browser.more-actions"
          aria-label={`More actions for ${target.name}`}
        >
          More
        </summary>
        <div className="gb-more-actions">
          {actions
            .filter((action) => action !== step.action)
            .map((action) => {
              const option = actionStep(target, action);
              return (
                <button
                  key={action}
                  data-testid="git-browser.action"
                  data-item-id={action}
                  type="button"
                  className="mini"
                  data-action={`git-browser-${action}`}
                  disabled={option.reason !== null}
                  title={option.reason ?? option.label}
                  onClick={() => {
                    onAction(target, action);
                  }}
                >
                  {option.label}
                </button>
              );
            })}
        </div>
      </details>
      {step.reason !== null && (
        <span className="gb-reason" id={reasonId}>
          {step.reason}
        </span>
      )}
    </div>
  );
}
