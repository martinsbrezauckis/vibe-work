/**
 * The plan's progress header ("Plan = view"): app chrome above the plan page, not part of it.
 * Progress comes from the page's step markup; while a turn runs Vibe's todo list is a live overlay ("live: step 3");
 * when the turn has ended and the todo list is ahead of the page, it says "Plan not updated" and offers a chip that
 * asks Vibe to update the page (the user still sends it). Nothing is enforced.
 * Depends on: model.ts, protocol types, transcript todos. Called by: PagesTab.
 * Invariants: renders only; page text (step text) is plain text.
 */
import { Button } from "../../components/Controls";
import type { ReactElement } from "react";

import type { Step } from "../../protocol/core";
import type { Todo } from "../transcript/model/todos";
import { liveStep, planMismatch, planProgress, type PlanMismatch } from "./model";

/** The header for a plan page with `steps`. */
export function PlanHeader({
  title,
  steps,
  todos,
  running,
  askUpdate,
}: {
  title: string;
  steps: readonly Step[];
  todos: readonly Todo[] | null;
  running: boolean;
  askUpdate: (mismatch: PlanMismatch) => void;
}): ReactElement {
  const progress = planProgress(steps);
  const live = running ? liveStep(steps, todos) : -1;
  const mismatch = planMismatch(steps, todos, running);
  const state =
    mismatch !== null
      ? "stale"
      : running
        ? "running"
        : progress.done === progress.total
          ? "done"
          : "idle";
  return (
    <div className="pages-planhead" data-state={state} aria-label="Plan progress">
      <div className="pages-planhead-row">
        <b>Plan</b>
        <span className="pages-planhead-title">{title}</span>
        <span
          className="pages-planhead-sub"
          data-progress={`${String(progress.done)}/${String(progress.total)}`}
        >
          {running ? "In progress" : "Turn ended"} · the page says {progress.done} of{" "}
          {progress.total} done
        </span>
        {live >= 0 && (
          <span className="pages-planhead-live" title="From Vibe's todo list while the turn runs">
            live: step {live + 1}
          </span>
        )}
      </div>
      <div className="pages-pbar" aria-hidden="true">
        {steps.map((step, i) => (
          <i
            key={step.id}
            data-status={step.status}
            data-live={i === live}
            title={`${step.id}: ${step.text} (${step.status})`}
          />
        ))}
      </div>
      {mismatch !== null && (
        <div className="pages-stale" role="status">
          <span>
            <b>Plan not updated.</b> For linked steps, Vibe&apos;s todo list says{" "}
            {mismatch.todoDone} of {mismatch.total} done, the page says {mismatch.pageDone} of{" "}
            {mismatch.total}.
          </span>
          <Button
            data-testid="pages.plan-header-ask-update"
            type="button"
            appearance="page"
            onClick={() => {
              askUpdate(mismatch);
            }}
          >
            Ask Vibe to update the page
          </Button>
        </div>
      )}
    </div>
  );
}
