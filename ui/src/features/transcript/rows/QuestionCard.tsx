/**
 * The question card: Vibe's `user_input` callback (`UserQuestionRequest`): each question
 * with its options, multi-select when the question says so, free text ("Other") unless hidden, the
 * footer note, Submit and Skip. The answer is Vibe's `UserInputCallbackOutput` (`model/callbacks.ts`).
 * New (ShellX has no question card). Same answerability rule as the approval card.
 * Depends on: model/callbacks, the transcript store, the core store. Called by: RowView.
 * Invariants: Submit stays disabled until every question has an answer; picks live in component
 * state (the card is the newest row while open, so it is not virtualised away in practice).
 */
import { type ReactElement, useState } from "react";
import { useFirstSight } from "../../../hooks/useFirstSight";
import { Selector } from "../../../components/Selector";

import { useApp } from "../../shell/AppContext";
import { useCoreState } from "../../shell/hooks";
import { useHeldBy } from "../../sessions/SessionHeldNote";
import { useTranscriptUi } from "../hooks";
import {
  type Picked,
  type Question,
  questionCancelOutput,
  questionOutput,
  questionsOf,
} from "../model/callbacks";
import { type Obj, entryId, str } from "../model/entries";

function QuestionField({
  question,
  name,
  picked,
  disabled,
  onChange,
}: {
  question: Question;
  name: string;
  picked: Picked;
  disabled: boolean;
  onChange: (next: Picked) => void;
}): ReactElement {
  const toggle = (label: string) => {
    if (picked.labels.includes(label))
      onChange({ ...picked, labels: picked.labels.filter((l) => l !== label) });
    else onChange({ ...picked, labels: [...picked.labels, label] });
  };
  return (
    <fieldset data-item-id={name} className="q" disabled={disabled}>
      <legend>
        {question.header !== "" ? <span className="q-tag">{question.header}</span> : null}
        {question.question}
      </legend>
      {question.multiSelect ? (
        question.options.map((option) => (
          <label data-item-id={`${name}:${option.label}`} key={option.label} className="q-opt">
            <input
              data-testid="transcript.question-card-toggle"
              type="checkbox"
              aria-label={option.label}
              title={option.label}
              name={name}
              checked={picked.labels.includes(option.label)}
              onChange={() => {
                toggle(option.label);
              }}
            />
            <span className="q-label">{option.label}</span>
            {option.description !== "" ? (
              <span className="q-desc">{option.description}</span>
            ) : null}
          </label>
        ))
      ) : (
        <Selector
          testId="transcript.question-card-on-change"
          itemId={name}
          label={question.question}
          value={picked.labels[0] ?? ""}
          disabled={disabled}
          options={[
            {
              value: "",
              label: picked.other === "" ? "Choose an answer" : "Other (typed)",
              disabled: true,
            },
            ...question.options.map((option) => ({
              value: option.label,
              label: option.label,
              ...(option.description === "" ? {} : { description: option.description }),
            })),
          ]}
          onChange={(value) => {
            onChange({ labels: [value], other: "" });
          }}
        />
      )}
      {question.allowOther ? (
        <label className="q-other">
          <span>Other</span>
          <input
            data-testid="transcript.question-card-on-change-2"
            type="text"
            aria-label="Other"
            title="Other"
            value={picked.other}
            maxLength={4000}
            onChange={(event) => {
              onChange(
                question.multiSelect
                  ? { ...picked, other: event.target.value }
                  : { labels: [], other: event.target.value },
              );
            }}
          />
        </label>
      ) : null}
    </fieldset>
  );
}

/** Renders an open question. */
export function QuestionCard({
  entry,
  sessionId,
}: {
  entry: Obj;
  sessionId: string;
}): ReactElement {
  const { transcript } = useApp();
  const callbackId = str(entry["callbackId"]) ?? "";
  const { questions, footer } = questionsOf(entry);
  const [picks, setPicks] = useState<Picked[]>(() =>
    questions.map(() => ({ labels: [], other: "" })),
  );
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
  const output = questionOutput(questions, picks);
  const send = (value: typeof output) => {
    if (disabled || value === null) return;
    void transcript.answer(sessionId, callbackId, value, "window");
  };
  // the card rises in and sends one pulse the first time it is on screen, then holds still.
  const fresh = useFirstSight(`card:${callbackId}`);
  return (
    <div
      className={`approval qcard${fresh ? " arrive" : ""}`}
      role="group"
      aria-label={`Question from Vibe: ${questions[0]?.question ?? ""}`}
      data-callback-id={callbackId}
      data-entry-id={entryId(entry)}
    >
      <div className="ah">
        <span className="px needs" aria-hidden="true" />
        <b>Vibe asks</b>
      </div>
      {questions.map((question, i) => (
        <QuestionField
          key={i}
          question={question}
          name={`q-${callbackId}-${String(i)}`}
          picked={picks[i] ?? { labels: [], other: "" }}
          disabled={disabled}
          onChange={(next) => {
            setPicks((all) => all.map((p, j) => (j === i ? next : p)));
          }}
        />
      ))}
      {footer !== null && footer !== "" ? <p className="q-footer">{footer}</p> : null}
      <div className="choices">
        <button
          data-testid="transcript.question-card-send"
          type="button"
          className="cbtn go"
          aria-label="Submit answers"
          title="Submit answers"
          disabled={disabled || output === null}
          onClick={() => {
            send(output);
          }}
        >
          Submit
        </button>
        <button
          data-testid="transcript.question-card-send-2"
          type="button"
          className="cbtn"
          aria-label="Skip questions"
          title="Skip questions"
          disabled={disabled}
          onClick={() => {
            send(questionCancelOutput());
          }}
        >
          Skip
        </button>
      </div>
      <p className="card-state" role="status">
        {sending
          ? "Sending your answer…"
          : problem !== undefined
            ? problem.message
            : offered
              ? ""
              : "Vibe has not offered this question for an answer."}
      </p>
    </div>
  );
}
