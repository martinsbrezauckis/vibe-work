/**
 * Page inputs, the presentation model (Handbook Format 1.2 §6.6): what each control shows from an input and
 * its stored answer, the value a control change sends, rank moves, and the one chip "Send to Vibe" puts into the
 * composer with every answer of the page. Pure functions, so the components only render.
 * Depends on: protocol/core (Input, InputAnswer, PageAnswers, AnswerValue), model.ts (Chip). Called by: AnswersPanel,
 * usePagesTab, pagesRoutes (Debug API), UI tests.
 * Invariants: option ids and labels come from the page FILE through the core (it refuses an answer that does not fit
 * the file); the chip is data that quotes the page, never a command.
 */
import type { AnswerValue, InputAnswer, PageAnswers } from "../../protocol/core";
import type { Chip } from "./model";

/** The option ids a control shows as chosen, ticked or in rank order (rank: the page's order until answered). */
export function chosen(item: InputAnswer): string[] {
  const value = item.answer?.value;
  if (value === undefined) {
    return item.input.kind === "rank" ? item.input.options.map((o) => o.id) : [];
  }
  switch (value.type) {
    case "choice":
      return [value.option];
    case "checklist":
      return value.checked;
    case "rank":
      return value.order;
    case "text":
      return [];
  }
}

/** The value a checklist sends when `option` is ticked (`on`) or unticked. */
export function toggled(item: InputAnswer, option: string, on: boolean): AnswerValue {
  const now = new Set(chosen(item));
  if (on) now.add(option);
  else now.delete(option);
  return {
    type: "checklist",
    checked: item.input.options.map((o) => o.id).filter((id) => now.has(id)),
  };
}

/** The rank after moving the option at `index` by `delta` (-1 up, +1 down); unchanged at the ends. */
export function moved(item: InputAnswer, index: number, delta: -1 | 1): AnswerValue {
  const order = chosen(item);
  const to = index + delta;
  if (to >= 0 && to < order.length) {
    const [picked] = order.splice(index, 1);
    if (picked !== undefined) order.splice(to, 0, picked);
  }
  return { type: "rank", order };
}

/** The label of option `id` (the id itself when the page no longer has it). */
export function labelOf(item: InputAnswer, id: string): string {
  return item.input.options.find((o) => o.id === id)?.label ?? id;
}

/** The ids (or the text) of a stored value, for an answer whose input is gone. */
function valueWords(value: AnswerValue): string[] {
  switch (value.type) {
    case "choice":
      return [value.option];
    case "checklist":
      return value.checked;
    case "rank":
      return value.order;
    case "text":
      return [value.text];
  }
}

/** One line per input for the chip ("- #id Question: answer"). */
function line(item: InputAnswer): string {
  const question = item.input.question === "" ? item.input.id : item.input.question;
  const words = item.words;
  let answer: string;
  if (item.answer === null) answer = "(not answered)";
  else if (item.input.kind === "rank")
    answer = words.map((w, i) => `${String(i + 1)}. ${w}`).join("; ");
  else if (item.input.kind === "text") answer = `"${words.join(" ")}"`;
  else answer = words.length === 0 ? "(none ticked)" : words.join("; ");
  return `- #${item.input.id} ${question}: ${answer}`;
}

/**
 * The chip "Send to Vibe" inserts: every input of the page with its answer (and detached answers), in the same words
 * the agent's `handbook_answers` tool uses, so the agent can match them.
 */
export function answersChip(folder: string, page: PageAnswers): Chip {
  const lines = [
    `Answers to the inputs on the page ${folder}/${page.page} ("${page.title ?? page.page}"):`,
    ...page.inputs.map(line),
    ...page.detached.map(
      (a) =>
        `- #${a.input} (no longer on the page) ${a.question}: ${valueWords(a.value).join("; ")}`,
    ),
    "Read them any time with handbook_answers; record the outcome in the page itself.",
  ];
  return { label: `Answers · ${page.page}`, source: "page", text: lines.join("\n") };
}
