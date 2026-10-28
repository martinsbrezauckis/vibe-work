/** Renders Vibe question fields and Submit/Skip through the shared AskCard and transcript client. */
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
            aria-label="Submit answers"
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
            aria-label="Skip questions"
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
      {footer !== null && footer !== "" ? <p className="q-footer">{footer}</p> : null}
    </AskCard>
  );
}
