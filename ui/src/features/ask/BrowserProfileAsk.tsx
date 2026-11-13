/**
 * Which browser profile a chat uses, asked on a card when the core cannot decide: several paired profiles and no
 * default, or a default whose window is closed. One button per connected profile shows the browser's own profile
 * name with the signed-in account under it (not repeated when it equals the name); a one-line note says when the
 * default or other paired profiles are not offered because their windows are closed; "Don't use <browser>" ends the
 * agent's open. The answer names the picked profile and the core selects it as the person's choice. The account is
 * screen text only: the core never passes it to the agent. Once the core's ask closes, the card becomes its audit
 * row, which names the picked profile (the core keeps it on the card as `picked`). Depends on: AskCard, AuditRow, the
 * transcript and core stores. Called by: RowView.
 */
import { type ReactElement } from "react";
import { Button } from "../../components/Controls";
import { TEXT, wording } from "../../vocabulary";
import type { AskRecord } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { useHeldBy } from "../sessions/SessionHeldNote";
import { useTranscriptUi } from "../transcript/hooks";
import { approvalEffect } from "../transcript/model/callbacks";
import { type Obj, arr, entryId, obj, str } from "../transcript/model/entries";
import { AskCard } from "./AskCard";
import { AuditText } from "./AuditRow";
import { type AuditLine, auditLine, whoAnswered } from "./model";
import { useCardClose } from "./useCardClose";
import { useWho } from "./useWho";
import { visibleText } from "./visibleText";

/** One offered profile. */
interface Choice {
  profileId: string;
  name: string;
  account: string | null;
}

/** The offered profiles of a browser profile card. */
export function profileChoices(entry: Obj): Choice[] {
  return arr(approvalEffect(entry)["choices"]).flatMap((row) => {
    const choice = obj(row);
    const profileId = str(choice?.["profileId"]);
    const name = str(choice?.["name"]);
    return profileId === null || name === null
      ? []
      : [{ profileId, name, account: str(choice?.["account"]) }];
  });
}

/** The account to show under a profile's name: none when it only repeats the name. */
function shownAccount(choice: Choice): string | null {
  return choice.account?.toLowerCase() === choice.name.toLowerCase() ? null : choice.account;
}

/** The card's note about paired profiles it does not offer because their windows are closed. */
function closedNote(effect: Obj, browser: string): string | null {
  const closedDefault = str(effect["closedDefault"]);
  if (closedDefault !== null)
    return wording(TEXT.browserProfileDefaultClosed, { profile: visibleText(closedDefault) });
  return effect["closedOthers"] === true
    ? wording(TEXT.browserProfileOthersClosed, { browser })
    : null;
}

/**
 * The audit row of a closed profile card, in plain words, wherever it is shown (the card itself or the turn's
 * steps group); other outcomes keep the shared audit wording.
 */
export function profileAuditLine(
  record: AskRecord,
  entry: Obj,
  via: "window" | "uiControl" | undefined,
): AuditLine | null {
  const browser = str(approvalEffect(entry)["browser"]) ?? "Chrome";
  const who = whoAnswered(record, via);
  const outcome = record.outcome;
  if (outcome.type === "allowed") {
    const picked = str(approvalEffect(entry)["picked"]);
    const name = profileChoices(entry).find((choice) => choice.profileId === picked)?.name;
    return name === undefined
      ? { square: "ok", text: `Picked the ${browser} profile for this chat · ${who}` }
      : {
          square: "ok",
          text: `${wording(TEXT.browserProfilePicked, { browser, profile: visibleText(name) })} · ${who}`,
        };
  }
  if (outcome.type === "refused" && outcome.by !== "core")
    return {
      square: "warn",
      text: `${wording(TEXT.browserProfileDeclined, { browser })} · ${who}`,
    };
  if (outcome.type === "cancelled") {
    const none = wording(TEXT.browserProfileNotPicked, { browser });
    return {
      square: "warn",
      text:
        outcome.reason === "the turn was stopped"
          ? `Stopped: ${none}`
          : `${none} (${outcome.reason})`,
    };
  }
  return auditLine(record, `${browser} profile for this chat`, via);
}

/** True for the core's browser profile card. */
export function isProfileCard(entry: Obj): boolean {
  return approvalEffect(entry)["kind"] === "browserProfile";
}

/** Renders an open browser profile card, or its audit row once the core recorded the outcome. */
export function BrowserProfileAsk({
  entry,
  sessionId,
  hostSessionId,
}: {
  entry: Obj;
  sessionId: string;
  hostSessionId: string;
}): ReactElement | null {
  const { transcript } = useApp();
  const callbackId = str(entry["callbackId"]) ?? "";
  const askId = `callback:${callbackId}`;
  const ask = useCoreState((s) => s.asks[askId]);
  const record = useCoreState((s) => s.askRecords[askId]);
  const offered = ask !== undefined;
  const sending = useTranscriptUi((s) => s.sending[callbackId] === true);
  const problem = useTranscriptUi((s) => s.problems[callbackId]);
  const via = useTranscriptUi((s) => s.answered[callbackId]);
  const held = useHeldBy(sessionId) !== null;
  const who = useWho("agent", hostSessionId, sessionId);
  const browser = str(approvalEffect(entry)["browser"]) ?? "Chrome";
  const note = closedNote(approvalEffect(entry), browser);
  const closed = record !== undefined && ask === undefined && !sending;
  const line = record === undefined ? null : profileAuditLine(record, entry, via);
  const focusInside = useCardClose(closed, hostSessionId, closed ? (line?.text ?? null) : null);
  if (closed)
    return line === null ? null : (
      <AuditText line={line} callbackId={callbackId} entryId={entryId(entry)} />
    );
  const disabled = held || !offered || sending || problem?.kind === "refused";
  const send = (output: { profileId?: string; decision: string }) => {
    focusInside(true);
    void transcript.answer(
      sessionId,
      callbackId,
      output.profileId === undefined
        ? { type: "approval", decision: { type: output.decision } }
        : {
            type: "approval",
            decision: { type: output.decision },
            profileId: output.profileId,
          },
      "window",
    );
  };
  return (
    <AskCard
      id={callbackId}
      className="qcard profile-card"
      question={`Which ${browser} profile should this chat use?`}
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
              : TEXT.askNotOffered
      }
      data={{ "data-callback-id": callbackId, "data-entry-id": entryId(entry) }}
      actions={
        <>
          <span className="profile-choices" role="group" aria-label={`${browser} profiles`}>
            {profileChoices(entry).map((choice) => (
              <Button
                key={choice.profileId}
                type="button"
                appearance="choice"
                data-testid="ask.browser-profile-choice"
                data-item-id={choice.profileId}
                aria-description={shownAccount(choice) ?? undefined}
                disabled={disabled}
                onClick={() => {
                  send({ decision: "approve", profileId: choice.profileId });
                }}
              >
                <span className="profile-choice-name">{visibleText(choice.name)}</span>
                {shownAccount(choice) === null ? null : (
                  <span className="profile-choice-account">
                    {visibleText(shownAccount(choice) ?? "")}
                  </span>
                )}
              </Button>
            ))}
          </span>
          <Button
            data-testid="ask.browser-profile-no"
            type="button"
            appearance="choice"
            disabled={disabled}
            onClick={() => {
              send({ decision: "deny" });
            }}
          >
            {wording(TEXT.browserProfileDecline, { browser })}
          </Button>
        </>
      }
    >
      {note === null ? null : <p className="q-footer profile-closed-note">{note}</p>}
      <p className="q-footer">
        The chat keeps this profile. A default profile in Settings &gt; Browser skips this question.
      </p>
    </AskCard>
  );
}
