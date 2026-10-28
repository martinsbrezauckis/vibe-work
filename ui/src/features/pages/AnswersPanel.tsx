/**
 * The controls for a page's inputs (Handbook Format 1.2 §6.6: "the app shows the controls, keeps your
 * answers like comments"): one control per input the page FILE declares (pick one, tick any, put in order, a short
 * answer), the detached answers, and one "Send to Vibe" button that puts every answer of the page into the composer
 * as a single chip. By default only one line shows (how many questions, how many answered, Send to Vibe, Show
 * answers); the caller holds that choice. Answers are kept in Vibe Work's data, never in the repository (the panel
 * says so).
 * Depends on: inputs.ts (pure model), protocol types, vocabulary. Called by: PagesTab.
 * Invariants: renders only; questions and option labels are page text shown as plain text; every change calls
 * `answer` (the core checks it against the page file); rank moves are buttons, so they work from the keyboard.
 */
import { Button } from "../../components/Controls";
import { type ReactElement, memo, useId, useState } from "react";

import { Selector } from "../../components/Selector";
import type { AnswerValue, InputAnswer, PageAnswers } from "../../protocol/core";
import { TEXT, wording } from "../../vocabulary";
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
    <div data-item-id={item.input.id} className="pages-answer-text">
      <textarea
        data-testid="pages.answers-panel-set-text"
        id={id}
        aria-label={item.input.question || item.input.id}
        value={text}
        maxLength={2000}
        onChange={(event) => {
          setText(event.target.value);
        }}
      />
      <Button
        data-testid="pages.answers-panel-answer"
        type="button"
        appearance="page"
        disabled={text.trim() === stored.trim()}
        onClick={() => {
          answer(item.input.id, text.trim() === "" ? null : { type: "text", text });
        }}
      >
        Save answer
      </Button>
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
          <li data-item-id={`${input.id}:${id}`} key={id} data-option={id}>
            <span>{labelOf(item, id)}</span>
            <button
              data-testid="pages.answers-panel-send"
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
              data-testid="pages.answers-panel-send-2"
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
        testId="pages.answers-panel-send-3"
        itemId={input.id}
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
        <li data-item-id={`${input.id}:${option.id}`} key={option.id}>
          <label>
            <input
              data-testid="pages.answers-panel-send-4"
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

const InputBlock = memo(function InputBlock({
  item,
  answer,
}: {
  item: InputAnswer;
  answer: Answer;
}): ReactElement {
  const { input } = item;
  const known = ["choice", "checklist", "rank", "text"].includes(input.kind);
  return (
    <li
      data-item-id={item.input.id}
      className="pages-input"
      data-input={input.id}
      data-kind={input.kind}
    >
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
        <Button
          data-testid="pages.answers-panel-answer-2"
          type="button"
          appearance="page"
          onClick={() => {
            answer(input.id, null);
          }}
        >
          Clear
        </Button>
      )}
    </li>
  );
});

/** "3 questions · 2 answered" (and "· sent to Vibe" once every answer was sent). */
function countLine(page: PageAnswers): string {
  const n = page.inputs.length;
  const questions = wording(n === 1 ? TEXT.answersQuestionsOne : TEXT.answersQuestionsMany, {
    n: String(n),
  });
  const line = wording(TEXT.answersCount, { questions, answered: String(page.answered) });
  return page.answered > 0 && page.unsent === 0 ? `${line} · sent to Vibe` : line;
}

/**
 * The panel for `page` (the shown page's inputs and answers): one line (the counts, "Send to Vibe" and the
 * Show answers toggle), and every control while `shown`. The hidden controls stay mounted, so an unsaved short
 * answer survives hiding them.
 */
export function AnswersPanel({
  page,
  answer,
  send,
  shown,
  onShownChange,
}: {
  page: PageAnswers;
  answer: Answer;
  send: () => void;
  shown: boolean;
  onShownChange: (shown: boolean) => void;
}): ReactElement {
  const controls = useId();
  return (
    <section className="pages-answers" aria-label="Your input on this page">
      <div className="pages-cbox-row">
        <span className="pages-answers-count">{countLine(page)}</span>
        <Button
          data-testid="pages.answers-panel-pages-btn"
          type="button"
          appearance="page"
          variant="primary"
          disabled={page.answered === 0 && page.detached.length === 0}
          onClick={send}
        >
          Send to Vibe
        </Button>
        <Button
          data-testid="pages.answers-panel-toggle"
          type="button"
          appearance="page"
          aria-expanded={shown}
          aria-controls={controls}
          onClick={() => {
            onShownChange(!shown);
          }}
        >
          {shown ? TEXT.answersHide : TEXT.answersShow}
        </Button>
      </div>
      <div id={controls} hidden={!shown}>
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
        <p className="pages-note">
          All answers in one message. Saved in Vibe Work, not in the repo.
        </p>
      </div>
    </section>
  );
}
