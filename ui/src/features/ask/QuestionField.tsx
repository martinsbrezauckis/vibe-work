/**
 * One question of Vibe's `user_input` callback inside the AskCard: its options as a visible list (a radio group, or
 * checkboxes when it is multi-select) and free text ("Other") unless hidden.
 * Depends on: Badge, AnswerOptions, the callbacks model. Called by: QuestionAsk.
 */
import { type ReactElement, useId } from "react";
import { Badge } from "../../components/Controls";
import type { Picked, Question } from "../transcript/model/callbacks";
import { AnswerOptions } from "./AnswerOptions";

/** Renders one question's answer fields. */
export function QuestionField({
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
  const legend = useId();
  return (
    <fieldset data-item-id={name} className="q" disabled={disabled}>
      <legend id={legend}>
        {question.header !== "" ? (
          <Badge presentation="question-label">{question.header}</Badge>
        ) : null}
        {question.question}
      </legend>
      <AnswerOptions
        question={question}
        name={name}
        picked={picked}
        disabled={disabled}
        labelledBy={legend}
        onChange={onChange}
      />
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
