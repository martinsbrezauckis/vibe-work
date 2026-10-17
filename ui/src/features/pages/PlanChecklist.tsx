/**
 * The plan checklist beside the chat: a compact list of the
 * current plan's steps, derived from the plan page in the handbook (one source, no copy; "Plan = view"):
 * done and current steps, a progress bar, the step Vibe's todo list says is running (live while the agent works), and
 * a click on a step opens that step of the full plan page in the Project tab. The core watches the plan file, so an
 * edit by the agent shows here within about 2 s.
 * Depends on: store.ts (currentPlan), model.ts (progress, live step), transcript todos, shell hooks. Called by: the
 * `plan` panel tab (register.tsx).
 * Invariants: renders only; step text is page text shown as plain text; nothing here writes the plan.
 */
import { type ReactElement, useEffect, useMemo } from "react";

import type { JsonValue } from "../../protocol/core";
import { useStore } from "../../state/createStore";
import { statusType } from "../../state/sessionSummary";
import { useCoreState, useShellState } from "../shell/hooks";
import { latestTodos } from "../transcript/model/todos";
import { finished, liveStep, overlaySteps, planProgress } from "./model";
import type { Pages } from "./store";

const NONE: readonly JsonValue[] = [];

const MARK: Readonly<Record<string, string>> = {
  done: "✓",
  skipped: "–",
  doing: "●",
  blocked: "!",
  todo: "○",
};

function Checklist({
  pages,
  sessionId,
  openStep,
}: {
  pages: Pages;
  sessionId: string;
  openStep: (path: string, step: string) => void;
}): ReactElement {
  const plan = useStore(pages.store, (s) => s.plans[sessionId]);
  const unavailable = useStore(pages.store, (s) => s.unavailable);
  const entry = useCoreState((s) => s.sessions[sessionId]);
  const history = Array.isArray(entry?.state?.["history"]) ? entry.state["history"] : NONE;
  const todos = useMemo(() => latestTodos(history), [history]);
  const running = entry !== undefined && ["running", "blocked"].includes(statusType(entry.session));
  useEffect(() => {
    void pages.loadPlan(sessionId);
  }, [pages, sessionId]);
  // a core without Project Pages says so once, here, instead of a refusal on every view.
  if (unavailable !== null)
    return (
      <div data-item-id={sessionId} className="empty">
        {unavailable}.
      </div>
    );
  if (plan === undefined)
    return (
      <div data-item-id={sessionId} className="empty">
        Reading the plan…
      </div>
    );
  if (plan.path === null || plan.facts === null)
    return (
      <div data-item-id={sessionId} className="empty">
        This project has no plan page yet. Ask Vibe to plan the work as a handbook page.
      </div>
    );
  // while a turn runs, steps the todo list has finished show as finished before the page catches up.
  const steps = overlaySteps(plan.facts.steps, todos, running);
  const progress = planProgress(steps);
  const live = running ? liveStep(plan.facts.steps, todos) : -1;
  const path = plan.path;
  const percent = progress.total === 0 ? 0 : Math.round((progress.done / progress.total) * 100);
  return (
    <section data-item-id={sessionId} className="plan-checklist" aria-label="Plan checklist">
      <header className="plan-checklist-head">
        <b title={path}>{plan.facts.title ?? path}</b>
        <span data-progress={`${String(progress.done)}/${String(progress.total)}`}>
          {progress.done} of {progress.total} done{running ? " · working" : ""}
        </span>
        <div
          className="plan-checklist-bar"
          role="progressbar"
          aria-label="Plan checklist progress"
          aria-valuemin={0}
          aria-valuemax={progress.total}
          aria-valuenow={progress.done}
        >
          <i style={{ width: `${String(percent)}%` }} />
        </div>
      </header>
      {steps.length === 0 ? (
        <p className="empty">The plan page has no steps yet.</p>
      ) : (
        <ol className="plan-checklist-steps">
          {steps.map((step, i) => (
            <li data-item-id={step.id} key={step.id}>
              <button
                data-testid="pages.plan-checklist-open-step"
                type="button"
                data-step={step.id}
                data-status={step.status}
                data-live={i === live}
                aria-current={step.status === "doing" || i === live ? "step" : undefined}
                title={
                  step.fromTodo
                    ? `${step.status === "skipped" ? "Skipped" : "Done"} in Vibe's todo list; the plan page is updated at the end of the turn. Open step ${step.id}`
                    : `Open step ${step.id} of the plan page`
                }
                data-from-todo={step.fromTodo}
                onClick={() => {
                  openStep(path, step.id);
                }}
              >
                <span className="plan-checklist-mark" aria-hidden="true">
                  {MARK[step.status] ?? "○"}
                </span>
                <span className={finished(step.status) ? "plan-checklist-done" : undefined}>
                  {step.text}
                </span>
                {i === live && <span className="plan-checklist-live">live</span>}
                {step.fromTodo && <span className="plan-checklist-source">from todo list</span>}
              </button>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

/** The tab body: the selected session's plan checklist. */
export function PlanChecklist({
  pages,
  openStep,
}: {
  pages: Pages;
  openStep: (sessionId: string, path: string, step: string) => void;
}): ReactElement {
  const sessionId = useShellState((s) => s.selected);
  if (sessionId === null) return <div className="empty">Choose a session to see its plan.</div>;
  return (
    <Checklist
      key={sessionId}
      pages={pages}
      sessionId={sessionId}
      openStep={(path, step) => {
        openStep(sessionId, path, step);
      }}
    />
  );
}
