/**
 * The Answers panel under a handbook page: one line by default (the question count, how many are answered, "Send to
 * Vibe" and a "Show answers" toggle), the full controls when shown. Renders the panel alone with synthetic answers.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { type ReactElement, useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { AnswersPanel } from "../../../ui/src/features/pages/AnswersPanel";
import type { InputAnswer, PageAnswers } from "../../../ui/src/protocol/core";

const question = (
  id: string,
  text: string,
  answered: boolean,
): InputAnswer => ({
  input: { id, kind: "text", question: text, options: [] },
  answer: answered
    ? {
        page: "decisions/0001-db.html",
        input: id,
        question: text,
        value: { type: "text", text: "Yes" },
        updatedMs: 1,
        sentMs: null,
      }
    : null,
  words: [],
});

const PAGE: PageAnswers = {
  page: "decisions/0001-db.html",
  title: "Storage",
  answered: 2,
  unsent: 2,
  detached: [],
  inputs: [
    question("one", "Which database?", true),
    question("two", "Which extras?", true),
    question("three", "What first?", false),
  ],
};

/** The panel with its shown state held the way the Project tab holds it (outside the panel). */
function Harness({
  page,
  send,
}: {
  page: PageAnswers;
  send: () => void;
}): ReactElement {
  const [shown, setShown] = useState(false);
  return (
    <AnswersPanel
      page={page}
      answer={vi.fn()}
      send={send}
      shown={shown}
      onShownChange={setShown}
    />
  );
}

const toggle = () =>
  screen.getByRole("button", { name: /^(Show|Hide) answers$/ });

describe("the Answers panel line", () => {
  it("is one line by default: the counts, Send to Vibe and Show answers", () => {
    render(<Harness page={PAGE} send={vi.fn()} />);
    expect(screen.getByText("3 questions · 2 answered")).toBeVisible();
    expect(screen.getByRole("button", { name: "Send to Vibe" })).toBeEnabled();
    expect(toggle()).toHaveTextContent("Show answers");
    expect(toggle()).toHaveAttribute("aria-expanded", "false");
    const region = document.getElementById(
      toggle().getAttribute("aria-controls") ?? "",
    );
    expect(region).not.toBeNull();
    expect(region).not.toBeVisible();
    expect(
      screen.queryByRole("textbox", { name: "Which database?" }),
    ).toBeNull();
  });

  it("shows and hides the answers from the keyboard; focus stays on the toggle", async () => {
    const user = userEvent.setup();
    render(<Harness page={PAGE} send={vi.fn()} />);
    await user.tab();
    await user.tab();
    expect(toggle()).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(toggle()).toHaveTextContent("Hide answers");
    expect(toggle()).toHaveAttribute("aria-expanded", "true");
    expect(toggle()).toHaveFocus();
    expect(
      screen.getByRole("textbox", { name: "Which database?" }),
    ).toBeVisible();
    await user.keyboard(" ");
    expect(toggle()).toHaveAttribute("aria-expanded", "false");
    expect(toggle()).toHaveFocus();
    expect(
      screen.queryByRole("textbox", { name: "Which database?" }),
    ).toBeNull();
  });

  it("sends every answer from the collapsed line, enabled exactly as before", async () => {
    const user = userEvent.setup();
    const send = vi.fn();
    const view = render(<Harness page={PAGE} send={send} />);
    await user.click(screen.getByRole("button", { name: "Send to Vibe" }));
    expect(send).toHaveBeenCalledTimes(1);
    expect(toggle()).toHaveAttribute("aria-expanded", "false");
    view.rerender(<Harness page={{ ...PAGE, answered: 0 }} send={send} />);
    expect(screen.getByRole("button", { name: "Send to Vibe" })).toBeDisabled();
  });

  it("counts a question the page adds and stays collapsed", () => {
    const view = render(<Harness page={PAGE} send={vi.fn()} />);
    const one = {
      ...PAGE,
      inputs: [PAGE.inputs[0] as InputAnswer],
      answered: 1,
    };
    view.rerender(<Harness page={one} send={vi.fn()} />);
    expect(screen.getByText("1 question · 1 answered")).toBeVisible();
    const more = {
      ...PAGE,
      inputs: [...PAGE.inputs, question("four", "Anything else?", false)],
    };
    view.rerender(<Harness page={more} send={vi.fn()} />);
    expect(screen.getByText("4 questions · 2 answered")).toBeVisible();
    expect(toggle()).toHaveAttribute("aria-expanded", "false");
  });

  it("keeps an unsaved short answer while the answers are hidden", async () => {
    const user = userEvent.setup();
    render(<Harness page={PAGE} send={vi.fn()} />);
    await user.click(toggle());
    await user.type(
      screen.getByRole("textbox", { name: "What first?" }),
      "The API",
    );
    await user.click(toggle());
    await user.click(toggle());
    expect(screen.getByRole("textbox", { name: "What first?" })).toHaveValue(
      "The API",
    );
  });
});
