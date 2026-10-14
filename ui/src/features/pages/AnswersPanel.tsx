/**
 * The controls for a page's inputs (Handbook Format 1.2 §6.6: "the app shows the controls, keeps your
 * answers like comments"): one control per input the page FILE declares (pick one, tick any, put in order, a short
 * answer), the detached answers, and one "Send to Vibe" button that puts every answer of the page into the composer
 * as a single chip. Answers are kept in Vibe Work's data, never in the repository (the panel says so).
 * Depends on: inputs.ts (pure model), protocol types. Called by: PagesTab.
 * Invariants: renders only; questions and option labels are page text shown as plain text; every change calls
 * `answer` (the core checks it against the page file); rank moves are buttons, so they work from the keyboard.
 */
import { type ReactElement, useState } from "react";

import { Selector } from "../../components/Selector";
import type { AnswerValue, InputAnswer, PageAnswers } from "../../protocol/core";
import { chosen, labelOf, moved, toggled } from "./inputs";

type Answer = (input: string, value: AnswerValue | null) => void;

/** `item`'s answer with `value` (a stand-in answer when none is stored yet), for the pure model's helpers. */
function withValue(item: InputAnswer, value: AnswerValue): InputAnswer["answer"] {
  return {
    page: item.answer?.page ?? "",
    input: item.input.id,
    question: item.input.question,
    value,
    updatedMs: item.answer?.updatedMs ?? 0,
    sentMs: null,
  };
}

function TextAnswer({ item, answer }: { item: InputAnswer; answer: Answer }): ReactElement {
  const stored = item.answer?.value.type === "text" ? item.answer.value.text : "";
  // The parent keys this control by the stored answer, so a new stored answer starts a fresh draft.
  const [text, setText] = useState(stored);
  const id = `hb-answer-${item.input.id}`;
  return (
    <div className="pages-answer-text">
      <textarea
        id={id}
        aria-label={item.input.question || item.input.id}
        value={text}
        maxLength={2000}
        onChange={(event) => {
          setText(event.target.value);
        }}
      />
      <button
        type="button"
        className="pages-btn"
        disabled={text.trim() === stored.trim()}
        onClick={() => {
          answer(item.input.id, text.trim() === "" ? null : { type: "text", text });
        }}
      >
        Save answer
      </button>
    </div>
  );
}

function Options({ item, answer }: { item: InputAnswer; answer: Answer }): ReactElement {
  const { input } = item;
  // Shown at once (the core's answer arrives a moment later; the parent keys this by the stored answer, so the
  // stored value takes over when it changes, and a refused answer falls back to it).
  const [on, setOn] = useState(() => chosen(item));
  const send = (value: AnswerValue) => {
    setOn(chosen({ ...item, answer: withValue(item, value) }));
    answer(input.id, value);
  };
  if (input.kind === "rank") {
    return (
      <ol className="pages-rank" aria-label={`${input.question || input.id}: order`}>
        {on.map((id, index) => (
          <li key={id} data-option={id}>
            <span>{labelOf(item, id)}</span>
            <button
              type="button"
              className="ibtn"
              aria-label={`Move “${labelOf(item, id)}” up`}
              disabled={index === 0}
              onClick={() => {
                send(
                  moved(
                    { ...item, answer: withValue(item, { type: "rank", order: on }) },
                    index,
                    -1,
                  ),
                );
              }}
            >
              ↑
            </button>
            <button
              type="button"
              className="ibtn"
              aria-label={`Move “${labelOf(item, id)}” down`}
              disabled={index === on.length - 1}
              onClick={() => {
                send(
                  moved(
                    { ...item, answer: withValue(item, { type: "rank", order: on }) },
                    index,
                    1,
                  ),
                );
              }}
            >
              ↓
            </button>
          </li>
        ))}
      </ol>
    );
  }
  if (input.kind === "choice") {
    return (
      <Selector
        label={input.question || input.id}
        value={on[0] ?? ""}
        options={[
          { value: "", label: "Choose an answer", disabled: true },
          ...input.options.map((option) => ({ value: option.id, label: option.label })),
        ]}
        onChange={(value) => {
          send({ type: "choice", option: value });
        }}
      />
    );
  }
  return (
    <ul className="pages-options" role="group" aria-label={input.question || input.id}>
      {input.options.map((option) => (
        <li key={option.id}>
          <label>
            <input
              type="checkbox"
              name={`hb-${input.id}`}
              data-option={option.id}
              checked={on.includes(option.id)}
              onChange={(event) => {
                send(
                  toggled(
                    { ...item, answer: withValue(item, { type: "checklist", checked: on }) },
                    option.id,
                    event.target.checked,
                  ),
                );
              }}
            />
            {option.label}
          </label>
        </li>
      ))}
    </ul>
  );
}

function InputBlock({ item, answer }: { item: InputAnswer; answer: Answer }): ReactElement {
  const { input } = item;
  const known = ["choice", "checklist", "rank", "text"].includes(input.kind);
  return (
    <li className="pages-input" data-input={input.id} data-kind={input.kind}>
      <span className="pages-input-q">{input.question || input.id}</span>
      {!known ? (
        <span className="pages-note">
          This input type ({input.kind}) is from a newer handbook format.
        </span>
      ) : input.kind === "text" ? (
        <TextAnswer
          key={item.answer?.value.type === "text" ? item.answer.value.text : ""}
          item={item}
          answer={answer}
        />
      ) : (
        <Options key={JSON.stringify(item.answer?.value ?? null)} item={item} answer={answer} />
      )}
      {item.answer !== null && input.kind !== "text" && (
        <button
          type="button"
          className="pages-btn"
          onClick={() => {
            answer(input.id, null);
          }}
        >
          Clear
        </button>
      )}
    </li>
  );
}

/** The panel for `page` (the shown page's inputs and answers). */
export function AnswersPanel({
  page,
  answer,
  send,
}: {
  page: PageAnswers;
  answer: Answer;
  send: () => void;
}): ReactElement {
  const total = page.inputs.length;
  return (
    <section className="pages-answers" aria-label="Your input on this page">
      <details open>
        <summary>
          This page asks for your input · {page.answered} of {total} answered
          {page.answered > 0 && page.unsent === 0 && " · sent to Vibe"}
        </summary>
        <ul className="pages-input-list">
          {page.inputs.map((item) => (
            <InputBlock key={item.input.id} item={item} answer={answer} />
          ))}
        </ul>
        {page.detached.length > 0 && (
          <p className="pages-detached">
            {page.detached.length} earlier answer{page.detached.length === 1 ? "" : "s"} no longer
            fit the page (the input or an option changed); Send to Vibe still includes them.
          </p>
        )}
        <div className="pages-cbox-row">
          <button
            type="button"
            className="pages-btn go"
            disabled={page.answered === 0 && page.detached.length === 0}
            onClick={send}
          >
            Send to Vibe
          </button>
          <span className="pages-note">
            All answers in one message. Saved in Vibe Work, not in the repo.
          </span>
        </div>
      </details>
    </section>
  );
}
