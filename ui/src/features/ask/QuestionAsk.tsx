/**
 * Renders Vibe question fields and Submit/Skip through the shared AskCard and transcript client. Keys inside the card:
 * 1-9 pick that option (after the card's arrival grace); Enter on an option or the single-line Other field
 * submits once every question has an answer. All submit paths share focus return to the Message box.
 */
import { visibleText } from "./visibleText";
import { type ReactElement, useState } from "react";
import { Button } from "../../components/Controls";
import { TEXT } from "../../vocabulary";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { useHeldBy } from "../sessions/SessionHeldNote";
import { useTranscriptUi } from "../transcript/hooks";
import {
  type Picked,
  questionCancelOutput,
  questionOutput,
  questionsOf,
} from "../transcript/model/callbacks";
import { type Obj, entryId, str } from "../transcript/model/entries";
import { optionKey } from "./AnswerOptions";
import { AskCard } from "./AskCard";
import { QuestionField } from "./QuestionField";
import { useCardClose } from "./useCardClose";
import { useWho } from "./useWho";

/** Renders an open question. */
export function QuestionAsk({
  entry,
  sessionId,
  hostSessionId,
}: {
  entry: Obj;
  sessionId: string;
  hostSessionId: string;
}): ReactElement {
  const { transcript } = useApp();
  const callbackId = str(entry["callbackId"]) ?? "";
  const { questions, footer } = questionsOf(entry);
  const [picks, setPicks] = useState<Picked[]>(() =>
    questions.map(() => ({ labels: [], other: "" })),
  );
  const offered = useCoreState(
    (s) =>
      s.sessions[sessionId]?.openCallbackIds.includes(callbackId) === true ||
      s.subagentCards[callbackId]?.sessionId === sessionId,
  );
  const sending = useTranscriptUi((s) => s.sending[callbackId] === true);
  const problem = useTranscriptUi((s) => s.problems[callbackId]);
  const held = useHeldBy(sessionId) !== null;
  const who = useWho(sessionId === hostSessionId ? "agent" : "subagent", hostSessionId, sessionId);
  const focusInside = useCardClose(!offered && !sending, hostSessionId, null);
  const disabled = held || !offered || sending || problem?.kind === "refused";
  const output = questionOutput(questions, picks);
  const send = (value: typeof output) => {
    if (disabled || value === null) return;
    // WebKit pointer and accessibility clicks may leave DOM focus in the Browser pane.
    // An accepted local answer owns focus return even when its button never took focus.
    focusInside(true);
    void transcript.answer(sessionId, callbackId, value, "window");
  };
  return (
    <AskCard
      id={callbackId}
      className="qcard"
      question="Vibe asks"
      who={who.text}
      whoTitle={who.title}
      buttons={[]}
      disabled={disabled}
      keys={false}
      onKeys={(event, ready) => {
        const onAnswer =
          event.target instanceof HTMLInputElement &&
          event.target.closest(".q-opts, .q-other") !== null;
        if (event.key === "Enter" && onAnswer) {
          event.preventDefault();
          if (ready) send(output);
          return;
        }
        if (!/^[1-9]$/.test(event.key)) return;
        event.preventDefault();
        if (!ready || disabled) return;
        optionKey(event.currentTarget, event.key, event.target, questions, picks, (i, next) => {
          setPicks((all) => all.map((p, j) => (j === i ? next : p)));
        });
      }}
      onFocusInside={focusInside}
      state={
        sending
          ? TEXT.askSending
          : problem !== undefined
            ? problem.message
            : offered
              ? ""
              : "Vibe has not offered this question for an answer."
      }
      data={{ "data-callback-id": callbackId, "data-entry-id": entryId(entry) }}
      actions={
        <>
          <Button
            data-testid="transcript.question-card-send"
            type="button"
            appearance="choice"
            variant="primary"
            // Named by its visible word ("Click Submit" for voice control); the tooltip says more.
            title="Submit answers"
            disabled={disabled || output === null}
            onClick={() => {
              send(output);
            }}
          >
            Submit
          </Button>
          <Button
            data-testid="transcript.question-card-send-2"
            type="button"
            appearance="choice"
            title="Skip questions"
            disabled={disabled}
            onClick={() => {
              send(questionCancelOutput());
            }}
          >
            Skip
          </Button>
        </>
      }
    >
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
      {footer !== null && footer !== "" ? <p className="q-footer">{visibleText(footer)}</p> : null}
    </AskCard>
  );
}
