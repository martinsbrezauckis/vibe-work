/**
 * Chrome Control cards in the `ui/transcript` route, so the Debug API reaches every control of the card the way a
 * click does:
 * - `read` lists each card row with its question, buttons in order (keys), covers line, reason, `open`
 *   and the audit words once it closed ([`chromeRowJson`]);
 * - `chrome {sessionId?, askId, choice: once | session | always | no, note?}` presses that button (only when it is
 *   on the card); the answer is marked "via UI control" in the audit row and the bell.
 * Depends on: ChromeAsk (choices, question), model.ts, the App's stores. Called by: transcriptRoutes.
 * Invariant: the same store actions as the card's buttons; an unknown or closed card is `notFound`.
 */
import type { ChromeAskView } from "../../protocol/core";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import type { Obj } from "../transcript/model/entries";
import { chromeChoices, chromeQuestion } from "./ChromeAsk";
import { type AskChoice, alwaysControl, auditLine, buttonsFor, coversText } from "./model";

function viewOf(entry: Obj): ChromeAskView | null {
  const card = entry["chromeAsk"];
  return typeof card === "object" && card !== null && !Array.isArray(card)
    ? (card as unknown as ChromeAskView)
    : null;
}

/** The card row as drawn. */
export function chromeRowJson(app: App, entry: Obj): Record<string, unknown> {
  const view = viewOf(entry);
  if (view === null) return {};
  const core = app.core?.store.get();
  const ui = app.transcript.store.get();
  const ask = core?.asks[view.askId];
  const record = core?.askRecords[view.askId];
  const choices = chromeChoices(ask?.offers, ask?.alwaysSetting, view.covers);
  const audit =
    ask === undefined
      ? (auditLine(record, view.summary, ui.answered[view.askId], ui.always[view.askId])?.text ??
        null)
      : null;
  return {
    askId: view.askId,
    open: view.open,
    group: view.group,
    question: chromeQuestion(ask?.kind ?? record?.ask.kind),
    summary: view.summary,
    reason: view.reason,
    buttons: ask === undefined ? [] : buttonsFor(choices, alwaysControl(ask.alwaysSetting)),
    covers: choices.includes("session") ? coversText(view.covers) : null,
    audit,
    problem: ui.problems[view.askId] ?? null,
  };
}

/** `chrome {askId, choice, note?}`: presses one button of an open Chrome Control card. */
export async function chromeAnswer(
  app: App,
  input: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const askId = input["askId"];
  if (typeof askId !== "string" || askId === "")
    throw new UiControlError("invalidParams", "askId must be a non-empty string");
  const core = app.core?.store.get();
  const ask = core?.asks[askId];
  if (ask === undefined || !ask.kind.startsWith("chrome/ask/"))
    throw new UiControlError("notFound", `no open Chrome Control card ${askId}`);
  const covers =
    Object.values(core?.sessions ?? {})
      .flatMap((s) => (Array.isArray(s.state?.["history"]) ? s.state["history"] : []))
      .map((e) => (typeof e === "object" && e !== null && !Array.isArray(e) ? viewOf(e) : null))
      .find((v) => v?.askId === askId)?.covers ?? [];
  const offered = chromeChoices(ask.offers, ask.alwaysSetting, covers);
  const choice = input["choice"] as AskChoice;
  if (!offered.includes(choice))
    throw new UiControlError("invalidParams", `choice must be one of ${offered.join(", ")}`);
  const note = typeof input["note"] === "string" ? input["note"] : "";
  const always = alwaysControl(ask.alwaysSetting);
  const ok =
    choice === "always" && always !== null
      ? await app.transcript.answerChromeAlways(askId, always, "uiControl")
      : choice === "always"
        ? false
        : await app.transcript.answerChrome(askId, choice, note, "uiControl");
  return { ok, problem: app.transcript.store.get().problems[askId] ?? null };
}
