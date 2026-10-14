/**
 * One effect step inside a steps group (`.step`): a header button with Vibe's verb and target
 * (`Reading` `hello.txt`, settled `Read` `1 line from hello.txt`) and the expandable body.
 * The status class, name, target and expandable output use labels from Vibe's display fields and status from Vibe's state enum (not
 * guessed from free text), body per kind (`StepBody`).
 * Adapted from ShellX.
 * Depends on: model/entries, StepBody, Time, the transcript store. Called by: StepsGroup.
 * Invariants: memoised on the entry object (a patch to another step does not re-render this one); the
 * open state lives in the store (`step:<entry id>`).
 */
import { type ReactElement, memo } from "react";
import { Walker } from "../../../components/Walker";

import { useApp } from "../../shell/AppContext";
import { useTranscriptUi } from "../hooks";
import { type Obj, effectView, entryId } from "../model/entries";
import { StepBody } from "./StepBody";
import { formatElapsed } from "./Time";

/** The status class: `run`, `done`, `fail` or `cancel`. */
function stateClass(view: ReturnType<typeof effectView>): string {
  if (view.live) return "run";
  if (view.status === "cancelled" || view.status === "skipped") return "cancel";
  if (view.status === "failed" || view.success === false) return "fail";
  return "done";
}

const STATE_TEXT: Record<string, string> = {
  run: "running",
  done: "done",
  fail: "failed",
  cancel: "cancelled",
};

/** Renders one effect step. */
export const StepItem = memo(function StepItem({ entry }: { entry: Obj }): ReactElement {
  const { transcript } = useApp();
  const id = entryId(entry);
  const key = `step:${id}`;
  const open = useTranscriptUi((s) => s.open[key] === true);
  const view = effectView(entry);
  const state = stateClass(view);
  const label = `${open ? "Hide" : "Show"} details for ${view.verb} ${view.target}`;
  return (
    <div className={`step ${state}`} data-kind={view.kind || "generic"} data-entry-id={id}>
      <button
        type="button"
        className="step-h"
        aria-expanded={open}
        aria-label={label}
        title={label}
        onClick={() => {
          transcript.toggle(key);
        }}
      >
        <span className="verb">{view.verb}</span>
        <span className="target" title={view.target}>
          {view.target}
        </span>
        <span className="smeta">
          {view.live ? (
            <Walker />
          ) : view.durationMs !== null && view.durationMs >= 1 ? (
            formatElapsed(view.durationMs)
          ) : null}
          <span className="sr">{STATE_TEXT[state]}</span>
        </span>
      </button>
      {open ? <StepBody entry={entry} view={view} /> : null}
    </div>
  );
});
