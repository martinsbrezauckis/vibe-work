/**
 * One question of Vibe's `user_input` callback inside the AskCard: its options (a selector,
 * or checkboxes when it is multi-select), free text ("Other") unless hidden. Moved unchanged from the old question
 * card. Depends on: Badge, Selector, the callbacks model. Called by: QuestionAsk.
 */
import type { ReactElement } from "react";
import { Badge } from "../../components/Controls";
import { Selector } from "../../components/Selector";
import type { Picked, Question } from "../transcript/model/callbacks";

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
  const toggle = (label: string) => {
    if (picked.labels.includes(label))
      onChange({ ...picked, labels: picked.labels.filter((l) => l !== label) });
    else onChange({ ...picked, labels: [...picked.labels, label] });
  };
  return (
    <fieldset data-item-id={name} className="q" disabled={disabled}>
      <legend>
        {question.header !== "" ? (
          <Badge presentation="question-label">{question.header}</Badge>
        ) : null}
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
