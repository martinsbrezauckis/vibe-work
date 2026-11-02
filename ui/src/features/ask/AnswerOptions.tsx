/**
 * A question's answer options shown as a list inside the ask card: every option and its description visible at once,
 * one row each, a radio group for one answer and checkboxes for several. The option words stay selectable: dragging
 * over them to copy never picks an option; a click, a number key, the arrow keys or an accessibility press does.
 * Depends on: the callbacks model. Called by: QuestionField (rows) and QuestionAsk (number keys, `optionKey`).
 * Invariant: each option's accessible name is its visible label then its number key ("Red 1", see the input), its
 * description the accessible description, the key also `aria-keyshortcuts`.
 */
import { type KeyboardEvent, type MouseEvent, type ReactElement, useId } from "react";
import type { Picked, Question } from "../transcript/model/callbacks";

/** Options past the ninth have no number key. */
const KEYED = 9;

/** The number key of the option at `index`, or undefined past the ninth. */
function keyOf(index: number): string | undefined {
  return index < KEYED ? String(index + 1) : undefined;
}

/** The answer after choosing `label`: one answer replaces the pick (and any typed Other), several toggle it. */
export function pickOption(question: Question, picked: Picked, label: string): Picked {
  if (!question.multiSelect) return { labels: [label], other: "" };
  return picked.labels.includes(label)
    ? { ...picked, labels: picked.labels.filter((l) => l !== label) }
    : { ...picked, labels: [...picked.labels, label] };
}

/**
 * True when the click only ended a text selection inside the row (a drag over the words to copy them, or a double
 * click selecting a word): the row's label must not pass that click on to its option. A click with `detail` 0 (a
 * keyboard or accessibility press) is never a selection.
 */
function selectingText(event: MouseEvent<HTMLElement>): boolean {
  if (event.detail === 0) return false;
  const selection = window.getSelection();
  if (selection === null || selection.isCollapsed) return false;
  const row = event.currentTarget;
  return (
    (selection.anchorNode !== null && row.contains(selection.anchorNode)) ||
    (selection.focusNode !== null && row.contains(selection.focusNode))
  );
}

/** Renders the option rows of one question. */
export function AnswerOptions({
  question,
  name,
  picked,
  disabled,
  labelledBy,
  onChange,
}: {
  question: Question;
  name: string;
  picked: Picked;
  disabled: boolean;
  /** The question's own words (its legend), naming the radio group. */
  labelledBy: string;
  onChange: (next: Picked) => void;
}): ReactElement {
  const id = useId();
  const multi = question.multiSelect;
  return (
    // One answer: a radio group named by the question. Several: the question's fieldset is already their group.
    <div
      className="q-opts"
      role={multi ? undefined : "radiogroup"}
      aria-labelledby={multi ? undefined : labelledBy}
      data-option-list={multi ? "checkbox" : "radio"}
    >
      {question.options.map((option, index) => {
        const labelId = `${id}-${String(index)}`;
        const descId = option.description === "" ? undefined : `${labelId}-d`;
        const key = keyOf(index);
        const checked = picked.labels.includes(option.label);
        return (
          <label
            key={option.label}
            data-testid="transcript.question-card-option-row"
            className="q-opt"
            data-item-id={`${name}:${option.label}`}
            data-checked={checked ? "true" : undefined}
            onClick={(event) => {
              if (selectingText(event)) event.preventDefault();
            }}
          >
            <input
              data-testid="transcript.question-card-option"
              data-option={option.label}
              type={multi ? "checkbox" : "radio"}
              name={name}
              value={option.label}
              // "<label> <key>" ("Red 1"), unlike card buttons (words only): macOS Voice Control does not press a
              // web radio or checkbox by name; it clicks painted text, and only text no other element is named
              // exactly like. So the label's words ("Click Red") and the painted digit ("Click 2", the route for
              // long labels Voice Control does not take whole) stay in the tree, and the name starts with the label.
              aria-labelledby={key === undefined ? labelId : `${labelId} ${labelId}-k`}
              aria-describedby={descId}
              aria-keyshortcuts={key}
              checked={checked}
              disabled={disabled}
              onKeyDown={(event: KeyboardEvent<HTMLInputElement>) => {
                // A checkbox list moves with the arrow keys as a radio group does by itself.
                if (!multi) return;
                const step =
                  event.key === "ArrowDown" || event.key === "ArrowRight"
                    ? 1
                    : event.key === "ArrowUp" || event.key === "ArrowLeft"
                      ? -1
                      : 0;
                if (step === 0) return;
                event.preventDefault();
                moveFocus(event.currentTarget, step);
              }}
              onChange={() => {
                onChange(pickOption(question, picked, option.label));
              }}
            />
            {/* The painted words stay in the tree as the text voice control clicks ("Click Red"). Without a key the
                name would equal them exactly, so they are hidden then: voice control would see two "Red" and ask. */}
            <span
              className="q-label"
              id={labelId}
              aria-hidden={key === undefined ? "true" : undefined}
            >
              {option.label}
            </span>
            {key === undefined ? null : (
              <kbd className="q-key" id={`${labelId}-k`}>
                {key}
              </kbd>
            )}
            {descId === undefined ? null : (
              <span className="q-desc" id={descId} aria-hidden="true">
                {option.description}
              </span>
            )}
          </label>
        );
      })}
    </div>
  );
}

/** Focuses the next or previous option of the same list (wrapping), as a radio group's arrow keys do. */
function moveFocus(from: HTMLInputElement, step: number): void {
  const list = from.closest(".q-opts");
  if (list === null) return;
  const all = [...list.querySelectorAll<HTMLInputElement>("input:not(:disabled)")];
  const at = all.indexOf(from);
  all.at((at + step) % all.length)?.focus();
}

/**
 * A number key pressed on the card: picks that option of the question holding the focus (else the first question
 * still without an answer, else the first) and moves the focus to it. Returns false when the key is not an option
 * number for that question.
 */
export function optionKey(
  card: HTMLElement,
  key: string,
  target: EventTarget | null,
  questions: readonly Question[],
  picks: readonly Picked[],
  onChange: (index: number, next: Picked) => void,
): boolean {
  if (!/^[1-9]$/.test(key)) return false;
  const fields = [...card.querySelectorAll<HTMLElement>("fieldset.q")];
  const holding = target instanceof Node ? fields.findIndex((f) => f.contains(target)) : -1;
  const open = picks.findIndex((p) => p.labels.length === 0 && p.other.trim() === "");
  const at = holding >= 0 ? holding : open >= 0 ? open : 0;
  const question = questions[at];
  const option = question?.options[Number(key) - 1];
  if (question === undefined || option === undefined) return false;
  onChange(at, pickOption(question, picks[at] ?? { labels: [], other: "" }, option.label));
  const inputs = fields[at]?.querySelectorAll<HTMLInputElement>(".q-opts input");
  [...(inputs ?? [])].at(Number(key) - 1)?.focus({ preventScroll: true });
  return true;
}
