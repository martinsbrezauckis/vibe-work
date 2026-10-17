/**
 * The approval card: shows exactly what Vibe will run (the gated
 * effect's `display.summary` and its payload: the command, the file write or edit as a diff, else the
 * input) and offers Vibe's own choices. Keys 1..n pick a choice while the card has focus. A deny can
 * carry feedback for the model.
 * The card waits for Vibe before showing a choice as accepted, including after a failed answer.
 * Answerable only while the core holds the callback open (`openCallbackIds`): after a reopen an
 * approval can stay in the session's history although the server no longer takes an answer for it,
 * and the card says so instead of offering buttons that cannot work.
 * Adapted from ShellX.
 * Depends on: model (callbacks, diff, entries), DiffView, ShowAll, the transcript store, the core store.
 * Called by: RowView.
 */
import { type KeyboardEvent, type ReactElement, useState } from "react";
import { useFirstSight } from "../../../hooks/useFirstSight";
import { Button } from "../../../components/Controls";
import { Selector } from "../../../components/Selector";

import { useApp } from "../../shell/AppContext";
import { useCoreState } from "../../shell/hooks";
import { useHeldBy } from "../../sessions/SessionHeldNote";
import { useTranscriptUi } from "../hooks";
import {
  DECISION_LABEL,
  type Decision,
  approvalEffect,
  approvalOutput,
  offeredDecisions,
  offeredScopes,
} from "../model/callbacks";
import { editDiff, writeDiff } from "../model/diff";
import { type Obj, arr, detailOf, entryId, obj, str } from "../model/entries";
import { DiffView } from "./DiffView";
import { ShowAll } from "./ShowAll";

const SCOPE_LABEL: Record<string, string> = {
  exact: "This file only",
  directory_recursive: "This folder and below",
};

/** The payload preview: what exactly will run or change. */
function Payload({ entry }: { entry: Obj }): ReactElement | null {
  const effect = approvalEffect(entry);
  const input = obj(effect["input"]);
  const id = `cb:${entryId(entry)}`;
  // The diff helpers read `detail.input`; the gated effect has not run yet, so there is no output.
  const asEntry: Obj = { detail: effect, state: {} };
  switch (effect["kind"]) {
    case "shell":
      return <pre className="cmd">$ {str(input?.["command"]) ?? ""}</pre>;
    case "file_write": {
      const diff = writeDiff(asEntry);
      return diff === null ? null : <DiffView diff={diff} id={id} />;
    }
    case "file_edit": {
      const diff = editDiff(asEntry);
      return diff === null ? null : <DiffView diff={diff} id={id} />;
    }
    default: {
      const display = obj(effect["display"]);
      const text =
        str(display?.["content"]) || (input === null ? "" : JSON.stringify(input, null, 2));
      return text === "" ? null : <ShowAll id={`${id}:payload`} text={text} />;
    }
  }
}

/** Renders an open approval. */
export function ApprovalCard({
  entry,
  sessionId,
}: {
  entry: Obj;
  sessionId: string;
}): ReactElement {
  const { transcript } = useApp();
  const callbackId = str(entry["callbackId"]) ?? "";
  const effect = approvalEffect(entry);
  const display = obj(effect["display"]);
  const detail = detailOf(entry);
  const kind = str(effect["kind"]);
  const title = str(entry["title"]) ?? "";
  const plainTitle =
    kind === "file_edit" || kind === "file_write"
      ? "Allow this edit?"
      : kind === "shell"
        ? "Allow this command?"
        : title;
  const choices = offeredDecisions(entry);
  const scopes = offeredScopes(entry);
  const [scope, setScope] = useState<string | undefined>(scopes[0]);
  const [feedback, setFeedback] = useState("");
  // a subagent's card is open while the core model holds it (its child session is not loaded).
  const offered = useCoreState(
    (s) =>
      s.sessions[sessionId]?.openCallbackIds.includes(callbackId) === true ||
      s.subagentCards[callbackId]?.sessionId === sessionId,
  );
  const sending = useTranscriptUi((s) => s.sending[callbackId] === true);
  const problem = useTranscriptUi((s) => s.problems[callbackId]);
  const held = useHeldBy(sessionId) !== null;
  const disabled = held || !offered || sending || problem?.kind === "refused";
  const permissions = arr(detail["requiredPermissions"])
    .map((p) => str(obj(p)?.["label"]))
    .filter((p): p is string => p !== null && p !== "");

  const choose = (decision: Decision) => {
    if (disabled) return;
    void transcript.answer(
      sessionId,
      callbackId,
      approvalOutput(decision, scope, feedback),
      "window",
    );
  };
  const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
    if (
      event.target instanceof HTMLInputElement ||
      (event.target instanceof HTMLElement &&
        event.target.closest('[role="combobox"], [role="listbox"]')) ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey
    )
      return;
    const index = Number(event.key) - 1;
    const decision = Number.isInteger(index) ? choices[index] : undefined;
    if (decision !== undefined) {
      event.preventDefault();
      choose(decision);
    }
  };

  // the card rises in and sends one pulse the first time it is on screen, then holds still.
  const fresh = useFirstSight(`card:${callbackId}`);
  return (
    <div
      data-testid="transcript.approval-card-div"
      className={`approval${fresh ? " arrive" : ""}`}
      role="group"
      aria-label={`Approval needed: ${str(display?.["summary"]) ?? str(entry["title"]) ?? ""}`}
      data-callback-id={callbackId}
      data-entry-id={entryId(entry)}
      onKeyDown={onKey}
    >
      <div className="ah">
        <span className="px needs" aria-hidden="true" />
        <b>Approval needed</b>
        <span className="sub" title={title}>
          {plainTitle}
        </span>
      </div>
      <div className="why">
        <span className="summary">{str(display?.["summary"]) ?? ""}</span>
        {str(detail["reason"]) ? <span className="reason">{str(detail["reason"])}</span> : null}
        {permissions.length > 0 ? (
          <span className="perms">Applies to: {permissions.join(", ")}</span>
        ) : null}
      </div>
      <Payload entry={entry} />
      {scopes.length > 1 ? (
        <fieldset className="scopes">
          <legend>When allowing</legend>
          <Selector
            testId="transcript.approval-card-selector"
            label="When allowing"
            value={scope ?? scopes[0] ?? ""}
            options={scopes.map((offeredScope) => ({
              value: offeredScope,
              label: SCOPE_LABEL[offeredScope] ?? offeredScope,
            }))}
            onChange={setScope}
          />
        </fieldset>
      ) : null}
      <div className="choices">
        {choices.map((decision, i) => {
          const props = {
            "data-decision": decision,
            disabled,
            "aria-label": `${DECISION_LABEL[decision]} (press ${String(i + 1)})`,
            title: `${DECISION_LABEL[decision]} (press ${String(i + 1)})`,
          };
          const label = (
            <>
              {DECISION_LABEL[decision]} <kbd>{String(i + 1)}</kbd>
            </>
          );
          return decision === "deny" ? (
            <Button
              key={decision}
              data-testid="transcript.approval-card-choose"
              data-item-id={decision}
              {...props}
              onClick={() => {
                choose(decision);
              }}
              variant="danger"
              small
            >
              {label}
            </Button>
          ) : (
            <button
              key={decision}
              type="button"
              data-testid="transcript.approval-card-choose"
              data-item-id={decision}
              {...props}
              onClick={() => {
                choose(decision);
              }}
              className={`cbtn${decision === "approve" ? " go" : ""}`}
            >
              {label}
            </button>
          );
        })}
      </div>
      {choices.includes("deny") ? (
        <label className="feedback">
          <span>Note for Vibe if you deny (optional)</span>
          <input
            data-testid="transcript.approval-card-set-feedback"
            type="text"
            aria-label="Note for Vibe if you deny (optional)"
            title="Note for Vibe if you deny (optional)"
            value={feedback}
            maxLength={2000}
            disabled={disabled}
            onChange={(event) => {
              setFeedback(event.target.value);
            }}
          />
        </label>
      ) : null}
      <p className="card-state" role="status">
        {sending
          ? "Sending your answer…"
          : problem !== undefined
            ? problem.message
            : offered
              ? ""
              : "Vibe has not offered this request for an answer. If the turn does not go on, stop it."}
      </p>
    </div>
  );
}
