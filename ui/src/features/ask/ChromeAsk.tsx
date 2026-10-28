/**
 * A Chrome Control call above the user's permissions as the one AskCard: "Allow this in your Chrome?" (or "Close your tab?"), "Vibe in your Chrome · “{session}”", what
 * and where, the reason line naming the level or the row that made Vibe ask, the covers line of "Allow for this
 * session", the call's input, and Allow once / Allow for this session / Always: allow “{group}” / No as the core
 * offers them ("Always" only on the close-tabs card: no card raises the level). Once the ask closes the card becomes
 * its audit row.
 * Depends on: AskCard, AuditText, model.ts, ShowAll, the core and transcript stores. Called by: RowView.
 * Invariants: answerable only while the core holds the ask open; "Always" only for an ask whose `alwaysSetting`
 * names a known control; the agent never answers here.
 */
import { type ReactElement } from "react";

import type { ChromeAskView } from "../../protocol/core";
import { TEXT } from "../../vocabulary";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { useHeldBy } from "../sessions/SessionHeldNote";
import { useTranscriptUi } from "../transcript/hooks";
import { ShowAll } from "../transcript/rows/ShowAll";
import { AskCard } from "./AskCard";
import { AuditText } from "./AuditRow";
import { type AskChoice, alwaysControl, auditLine, buttonsFor } from "./model";
import { useCardClose } from "./useCardClose";
import { useWho } from "./useWho";

/** The card's question. */
export function chromeQuestion(kind: string | undefined): string {
  return kind === "chrome/ask/closeTabs" ? "Close your tab?" : "Allow this in your Chrome?";
}

/** The answers a Chrome Control card offers, in the fixed order (no session answer without covers words). */
export function chromeChoices(
  offers: readonly string[] | undefined,
  alwaysSetting: string | null | undefined,
  covers: ChromeAskView["covers"],
): AskChoice[] {
  const offered = offers ?? [];
  const out: AskChoice[] = [];
  if (offered.includes("once")) out.push("once");
  if (offered.includes("session") && covers.length > 0) out.push("session");
  if (offered.includes("once") && alwaysControl(alwaysSetting) !== null) out.push("always");
  if (offered.includes("no")) out.push("no");
  return out;
}

/** Renders one Chrome Control card, or its audit row once it closed. */
export function ChromeAsk({
  view,
  sessionId,
}: {
  view: ChromeAskView;
  sessionId: string;
}): ReactElement {
  const { transcript } = useApp();
  const id = view.askId;
  const ask = useCoreState((s) => s.asks[id]);
  const record = useCoreState((s) => s.askRecords[id]);
  const sending = useTranscriptUi((s) => s.sending[id] === true);
  const problem = useTranscriptUi((s) => s.problems[id]);
  const native = useTranscriptUi((s) => s.native[id]);
  const via = useTranscriptUi((s) => s.answered[id]);
  const done = useTranscriptUi((s) => s.always[id]);
  const held = useHeldBy(sessionId) !== null;
  const who = useWho("chrome", sessionId, sessionId);
  const closed = record !== undefined && ask === undefined && !sending;
  const line = closed ? auditLine(record, view.summary, via, done) : null;
  const focusInside = useCardClose(closed, sessionId, line?.text ?? null);
  if (closed) return line === null ? <></> : <AuditText line={line} entryId={`chrome:${id}`} />;

  const always = alwaysControl(ask?.alwaysSetting);
  const choices = chromeChoices(ask?.offers, ask?.alwaysSetting, view.covers);
  const disabled =
    held || ask === undefined || sending || native === "waiting" || problem?.kind === "refused";
  const choose = (choice: AskChoice) => {
    if (choice === "always") {
      if (always !== null) void transcript.answerChromeAlways(id, always, "window");
      return;
    }
    void transcript.answerChrome(id, choice, null, "window");
  };
  const input = JSON.stringify(view.input, null, 2);
  const state =
    native === "waiting"
      ? TEXT.askNativeWaiting
      : sending
        ? TEXT.askSending
        : (problem?.message ?? (native === "cancelled" ? TEXT.askNativeCancelled : ""));
  return (
    <AskCard
      id={id}
      question={chromeQuestion(ask?.kind)}
      who={who.text}
      whoTitle={who.title}
      summary={view.summary}
      reason={view.reason}
      covers={choices.includes("session") ? view.covers : null}
      buttons={buttonsFor(choices, always)}
      disabled={disabled}
      onChoose={choose}
      state={state}
      decisionOf={(choice) => choice}
      onFocusInside={focusInside}
      data={{ "data-ask-id": id, "data-entry-id": `chrome:${id}`, "data-chrome-ask": "" }}
    >
      {input === "{}" || input === "null" ? null : (
        <ShowAll id={`chrome:${id}:input`} text={input} />
      )}
    </AskCard>
  );
}
