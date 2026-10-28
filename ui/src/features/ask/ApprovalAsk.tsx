/**
 * Vibe's approval callback as the one AskCard: the question per kind, who asks, the exact
 * effect (`display.summary` and its payload: the command, the edit as a diff, else the input), the covers line from
 * the core's grant offer when "Allow for this session" is offered, and Allow once / Allow for this session /
 * Always: {control} / No as offered. Once the core's ask closes, the card is replaced by its audit row.
 * Depends on: AskCard, AuditRow, model.ts, the transcript model (callbacks, diff, entries), DiffView, ShowAll, the
 * transcript store and the core store. Called by: RowView.
 * Invariants: answerable only while the core holds the callback open (`openCallbackIds`, or a subagent's card in
 * `subagentCards`); "Stop the turn" and Vibe's "Always allow" are never offered; "Allow for this session" shows
 * only with its covers line; the session answer's path scope is
 * never chosen here (the core keeps exactly the permission Vibe sent).
 */
import { type ReactElement } from "react";

import type { JsonValue } from "../../protocol/core";
import { TEXT } from "../../vocabulary";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { useHeldBy } from "../sessions/SessionHeldNote";
import { useTranscriptUi } from "../transcript/hooks";
import { approvalEffect, approvalOutput } from "../transcript/model/callbacks";
import { editDiff, writeDiff } from "../transcript/model/diff";
import { type Obj, detailOf, entryId, obj, str } from "../transcript/model/entries";
import { DiffView } from "../transcript/rows/DiffView";
import { ShowAll } from "../transcript/rows/ShowAll";
import { AskCard } from "./AskCard";
import { AuditRow } from "./AuditRow";
import { useCardClose } from "./useCardClose";
import { useWho } from "./useWho";
import {
  type AskChoice,
  DECISION_OF,
  alwaysControl,
  approvalChoices,
  approvalQuestion,
  approvalSummary,
  auditLine,
  buttonsFor,
  isChromeControl,
} from "./model";

/** The payload preview: what exactly will run or change. */
function Payload({ entry }: { entry: Obj }): ReactElement | null {
  const effect = approvalEffect(entry);
  const input = obj(effect["input"]);
  const id = `cb:${entryId(entry)}`;
  // The diff helpers read `detail.input`; the gated effect has not run yet, so there is no output.
  const asEntry: Obj = { detail: effect, state: {} };
  switch (effect["kind"]) {
    case "shell":
      return <pre className="cmd">$ {str(input?.["command"]) ?? ""}</pre>;
    case "file_search":
      return <pre className="cmd">$ {str(effect["vwCommand"]) ?? "Command unavailable"}</pre>;
    case "file_write": {
      const diff = writeDiff(asEntry);
      return diff === null ? null : <DiffView diff={diff} id={id} />;
    }
    case "file_edit": {
      const diff = editDiff(asEntry);
      return diff === null ? null : <DiffView diff={diff} id={id} />;
    }
    default: {
      const display = obj(effect["display"]);
      const text =
        str(display?.["content"]) || (input === null ? "" : JSON.stringify(input, null, 2));
      return text === "" ? null : <ShowAll id={`${id}:payload`} text={text} />;
    }
  }
}

/** The card-state line of an approval. */
function stateText(o: {
  sending: boolean;
  problem: string | undefined;
  offered: boolean;
  native: "waiting" | "cancelled" | undefined;
}): string {
  if (o.native === "waiting") return TEXT.askNativeWaiting;
  if (o.sending) return TEXT.askSending;
  if (o.problem !== undefined) return o.problem;
  if (o.native === "cancelled") return TEXT.askNativeCancelled;
  return o.offered ? "" : TEXT.askNotOffered;
}

/** Renders an open approval, or its audit row once the core recorded the outcome. */
export function ApprovalAsk({
  entry,
  sessionId,
  hostSessionId,
}: {
  entry: Obj;
  /** The session the answer goes to (a subagent's child session for its card). */
  sessionId: string;
  /** The transcript's own session (the parent of a subagent). */
  hostSessionId: string;
}): ReactElement {
  const { core, transcript } = useApp();
  const callbackId = str(entry["callbackId"]) ?? "";
  const askId = `callback:${callbackId}`;
  const ask = useCoreState((s) => s.asks[askId]);
  const record = useCoreState((s) => s.askRecords[askId]);
  const covers = useCoreState(
    (s) => s.askGrants.offers.find((o) => o.callbackId === callbackId)?.covers,
  );
  const offered = useCoreState(
    (s) =>
      (entry["vwToolAsk"] === true && s.asks[askId] !== undefined) ||
      s.sessions[sessionId]?.openCallbackIds.includes(callbackId) === true ||
      s.subagentCards[callbackId]?.sessionId === sessionId,
  );
  const sending = useTranscriptUi((s) => s.sending[callbackId] === true);
  const problem = useTranscriptUi((s) => s.problems[callbackId]);
  const native = useTranscriptUi((s) => s.native[callbackId]);
  const held = useHeldBy(sessionId) !== null;
  const sourceSession = str(entry["vwSourceSession"]) ?? sessionId;
  const who = useWho(
    isChromeControl(entry) ? "chrome" : sourceSession === hostSessionId ? "agent" : "subagent",
    hostSessionId,
    sourceSession,
  );
  const summary = approvalSummary(entry);
  const via = useTranscriptUi((s) => s.answered[callbackId]);
  const alwaysDone = useTranscriptUi((s) => s.always[callbackId]);
  // The core records either Vibe's confirmation or an explicit unconfirmed outcome after the send finishes.
  const closed = record !== undefined && ask === undefined && !sending;
  const focusInside = useCardClose(
    closed,
    hostSessionId,
    closed ? (auditLine(record, summary, via, alwaysDone)?.text ?? null) : null,
    () => {
      // Read now, not at render: the card may already be gone when its answer's record arrives.
      const state = core?.store.get();
      const settled = state?.askRecords[askId];
      const ui = transcript.store.get();
      return settled !== undefined && state?.asks[askId] === undefined
        ? (auditLine(settled, summary, ui.answered[callbackId], ui.always[callbackId])?.text ??
            null)
        : null;
    },
  );
  if (closed) return <AuditRow record={record} summary={summary} callbackId={callbackId} />;

  const always = alwaysControl(ask?.alwaysSetting);
  const choices = approvalChoices(entry, ask?.offers, ask?.alwaysSetting, covers);
  const buttons = buttonsFor(choices, always);
  const disabled =
    held || !offered || sending || native === "waiting" || problem?.kind === "refused";
  const choose = (choice: AskChoice) => {
    const output: JsonValue = approvalOutput(DECISION_OF[choice]);
    if (choice === "always" && always !== null) {
      void transcript.answerAlways(sessionId, callbackId, always, output, "window");
      return;
    }
    void transcript.answer(sessionId, callbackId, output, "window");
  };
  return (
    <AskCard
      id={callbackId}
      question={approvalQuestion(entry)}
      who={who.text}
      whoTitle={who.title}
      summary={summary}
      reason={str(detailOf(entry)["reason"])}
      covers={choices.includes("session") ? (covers ?? null) : null}
      buttons={buttons}
      disabled={disabled}
      onChoose={choose}
      state={stateText({ sending, problem: problem?.message, offered, native })}
      decisionOf={(choice) => DECISION_OF[choice]}
      onFocusInside={focusInside}
      data={{ "data-callback-id": callbackId, "data-entry-id": entryId(entry) }}
    >
      <Payload entry={entry} />
    </AskCard>
  );
}
