/**
 * What the one AskCard says: the question per kind, who asks, the buttons in their fixed order with their keys,
 * the covers line from the core's words, and the one-line audit row of every outcome. Pure: the card components
 * render these, the
 * `ui/transcript` route and the bell (`state/askBell.ts`) use the same functions, so a click, a route action and a
 * notification can never describe an ask differently.
 * Depends on: the transcript model (entries, callbacks), protocol/core ask types, vocabulary.
 * Called by: AskCard and its adapters, AuditRow, transcriptRoutes, askBell, the grants list.
 * Invariants: "Always" is offered only for an ask whose `alwaysSetting` names a control in `ALWAYS_CONTROLS`,
 * never Vibe's `approve_permanently`; "Stop the turn" is never a card button (the composer's Stop does
 * it); a button exists only when its answer is offered, so a key is never bound to an absent button.
 */
import { builtinTool, toolFallback, toolText } from "../transcript/model/toolLabels";
import type { AskRecord, Command, CoversPart } from "../../protocol/core";
import { term } from "../../vocabulary";
import { type Decision, approvalEffect, offeredDecisions } from "../transcript/model/callbacks";
import { type Obj, arr, detailOf, obj, str } from "../transcript/model/entries";

/** One answer a card can offer, in the fixed order used by every card. */
export type AskChoice = "once" | "session" | "always" | "no";

/** The fixed button order. */
export const CHOICE_ORDER: readonly AskChoice[] = ["once", "session", "always", "no"];

/** A visible button: its answer, its text, its key (= its position, 1-based). */
export interface AskButton {
  choice: AskChoice;
  label: string;
  key: number;
}

/**
 * Settings controls an "Always" answer may set: the control's id in the core's policy row
 * (`Ask.alwaysSetting`), the words of the button and of the audit row, and the Settings place that shows it.
 */
export interface AlwaysControl {
  /** "turn on Close your tabs": the button reads "Always: {action}". */
  action: string;
  /** "Close your tabs": the control's name. */
  control: string;
  /** What the click did, for the audit row: "turned on “Close your tabs”" ("Allowed, and {done} in Settings"). */
  done: string;
  /** The control's state after the click, said plainly when the answer then did not reach Vibe. */
  after: string;
  /** The control's own Settings command (the core asks its own native confirmation where the control has one). */
  command: Command;
}

const closeTabs = term("browserPermissions", "closeUserTabs").name;

/** The controls Vibe Work owns that an "Always" may set. A setting id not listed here offers no "Always". */
export const ALWAYS_CONTROLS: Readonly<Record<string, AlwaysControl>> = {
  "browser-close-user-tabs": {
    action: `turn on ${closeTabs}`,
    control: closeTabs,
    done: `turned on “${closeTabs}”`,
    after: `“${closeTabs}” is on in Settings › Browser.`,
    command: {
      type: "browser",
      request: { op: "setPermissions", permissions: { closeUserTabs: true } },
    },
  },
};

/** The "Always" control an ask offers, or `null` (no setting, or a setting the window does not know). */
export function alwaysControl(setting: string | null | undefined): AlwaysControl | null {
  return setting !== null && setting !== undefined && Object.hasOwn(ALWAYS_CONTROLS, setting)
    ? (ALWAYS_CONTROLS[setting] ?? null)
    : null;
}

/**
 * Button text per answer. Kept here, not in `vocabulary.json`: as vocabulary terms the vocabulary check would make
 * every native consent dialog and agent tool text that says "Allow once" read the same entry, a wording change
 * across the app's native dialogs. One place in the window: this table.
 */
const CHOICE_LABEL: Readonly<Record<Exclude<AskChoice, "always">, string>> = {
  once: "Allow once",
  session: "Allow for this session",
  no: "No",
};

/** The text of `choice`'s button ("Always: {action}" names the control it sets). */
function choiceLabel(choice: AskChoice, always: AlwaysControl | null = null): string {
  return choice === "always" ? `Always: ${always?.action ?? ""}` : CHOICE_LABEL[choice];
}

/** The answers in fixed order with their keys; only offered ones. */
export function buttonsFor(
  offered: readonly AskChoice[],
  always: AlwaysControl | null,
): AskButton[] {
  return CHOICE_ORDER.filter((c) => offered.includes(c) && (c !== "always" || always !== null)).map(
    (choice, i) => ({ choice, label: choiceLabel(choice, always), key: i + 1 }),
  );
}

/** Vibe's decision behind each card answer ("always" answers Vibe once after the control is written). */
export const DECISION_OF: Record<AskChoice, Decision> = {
  once: "approve",
  session: "approve_for_session",
  always: "approve",
  no: "deny",
};

/**
 * The answers an approval card offers: Vibe's own choices as `offeredDecisions` filters them (no "Always allow",
 * "Allow for this session" only where the core can keep it and only once the core's covers words are here, so the
 * card always says how far it reaches), without "Stop the turn", plus "Always" when the core's ask names a known
 * control.
 */
export function approvalChoices(
  entry: Obj,
  offers: readonly string[] | undefined,
  alwaysSetting: string | null | undefined,
  covers: readonly CoversPart[] | null | undefined,
): AskChoice[] {
  const decisions = offeredDecisions(entry, offers);
  const out: AskChoice[] = [];
  if (decisions.includes("approve")) out.push("once");
  if (
    decisions.includes("approve_for_session") &&
    covers !== null &&
    covers !== undefined &&
    covers.length > 0
  )
    out.push("session");
  if (decisions.includes("approve") && alwaysControl(alwaysSetting) !== null) out.push("always");
  if (decisions.includes("deny")) out.push("no");
  return out;
}

const COMMAND_TOOLS = [
  "bash",
  "file_system.bash",
  "shell",
  "mcp_vw.process_start",
  "mcp_vw.secret_run",
];

/** True for a Chrome Control call (its tools are routed through Vibe Work's checked MCP gateway). */
export function isChromeControl(entry: Obj): boolean {
  const tool = str(approvalEffect(entry)["toolName"]) ?? "";
  return /^mcp_vw\.chrome_(status|task|execute)$/.test(tool);
}

/** A tool name as people read it: Vibe's `mcp_` route prefix dropped ("mcp_notes.list_notes" → "notes.list_notes"). */
function toolName(entry: Obj): string {
  const tool = str(approvalEffect(entry)["toolName"]) ?? str(entry["title"]) ?? "this tool";
  return tool.replace(/^mcp_/, "");
}

/** The card's question. */
export function approvalQuestion(entry: Obj): string {
  const effect = approvalEffect(entry);
  const kind = str(effect["kind"]);
  const tool = str(effect["toolName"]) ?? "";
  if (tool === "mcp_vw.document_read" && entry["vwToolAsk"] === true)
    return str(obj(effect["display"])?.["summary"]) ?? "Read this document?";
  if (isChromeControl(entry)) return "Allow this in your Chrome?";
  if (kind === "shell" || kind === "file_search" || COMMAND_TOOLS.includes(tool))
    return "Allow this command?";
  if (kind === "file_edit" || kind === "file_write") return "Allow this edit?";
  const outside = arr(detailOf(entry)["requiredPermissions"]).some(
    (p) => str(obj(p)?.["scope"]) === "outside_directory",
  );
  if (outside) return "Allow this outside the folder?";
  const known = builtinTool(tool);
  return `Allow ${known?.permission ?? toolName(entry)}?`;
}

/** The exact what (`display.summary`), or the entry's title. */
export function approvalSummary(entry: Obj): string {
  const effect = approvalEffect(entry);
  const tool = str(effect["toolName"]) ?? "";
  return toolText(
    tool,
    str(obj(effect["display"])?.["summary"]) ?? str(entry["title"]) ?? toolFallback(tool),
    str(obj(effect["input"])?.["command"]),
  );
}

/** Who asks, with the session title. */
export function whoAsks(
  requester: "agent" | "subagent" | "chrome" | "vibeWork",
  session: string,
  subagent?: string,
): string {
  const quoted = `“${session}”`;
  switch (requester) {
    case "agent":
      return `Vibe · ${quoted}`;
    case "subagent":
      return `Vibe's subagent “${subagent ?? "subagent"}” · ${quoted}`;
    case "chrome":
      return `Vibe in your Chrome · ${quoted}`;
    case "vibeWork":
      return `Vibe Work · ${quoted}`;
  }
}

/**
 * The covers words for the start of a row (the session grants list, the Allowed page): the first letter of the first
 * non-empty part upper-cased. A first part shown as code keeps its case (a command or a tool name is exact text).
 */
export function coversCapitalised(parts: readonly CoversPart[]): CoversPart[] {
  const out = parts.map((p) => ({ ...p }));
  const first = out.find((p) => p.text !== "");
  if (first !== undefined && !first.code)
    first.text = first.text.charAt(0).toUpperCase() + first.text.slice(1);
  return out;
}

/** Plain text of covers words (tooltips, the route, search). */
export function coversText(parts: readonly CoversPart[]): string {
  return parts.map((p) => p.text).join("");
}

/** Who answered, as a phrase. */
export function whoAnswered(
  record: AskRecord | undefined,
  via: "window" | "uiControl" | undefined,
): string {
  if (via === "uiControl") return "by the Debug API (UI control)";
  const outcome = record?.outcome;
  const by = outcome !== undefined && "by" in outcome ? outcome.by : undefined;
  if (by === "debugApi") return "via the Debug API";
  if (via === "window" || by === "person") return "by you";
  return "answered outside this window";
}

/** One audit row: its square and its words; `title` holds the full note when the row cuts it. */
export interface AuditLine {
  square: "ok" | "warn" | "err";
  text: string;
  title?: string;
}

const NOTE_MAX = 80;

/** The note part of a "Said No" row, cut to one line. */
function notePart(note: string | null | undefined): { text: string; title?: string } {
  if (note === null || note === undefined || note.trim() === "") return { text: "" };
  const clean = note.replace(/\s+/g, " ").trim();
  const cut = clean.length > NOTE_MAX ? `${clean.slice(0, NOTE_MAX - 1)}…` : clean;
  return { text: ` · Your note: “${cut}”`, ...(cut === clean ? {} : { title: clean }) };
}

/** What an "Always" click did (the window remembers it: the record says "allowed once"). */
export interface AlwaysDone {
  /** "turned on “Close your tabs”". */
  done: string;
  /** The control's state afterwards. */
  after: string;
}

/** The words of an outcome that closed the card without the user's yes . */
function closedWords(record: AskRecord): string | null {
  const outcome = record.outcome;
  switch (outcome.type) {
    case "cancelled":
      return outcome.reason === "the turn was stopped" ? "Stopped" : `Closed (${outcome.reason})`;
    case "refused":
      return "Not allowed";
    default:
      return null;
  }
}

/**
 * The audit row of a closed ask from the core's record. `always` is what an "Always" answer did. When the
 * click set the control but the card closed before the answer reached Vibe (it ran out while the confirmation was
 * open), the row says so and names the control's state: the control stays set. `null`: the record is not here yet.
 */
export function auditLine(
  record: AskRecord | undefined,
  summary: string,
  via: "window" | "uiControl" | undefined,
  always?: AlwaysDone,
): AuditLine | null {
  if (record === undefined) return null;
  const who = whoAnswered(record, via);
  const outcome = record.outcome;
  const late = always === undefined ? null : closedWords(record);
  if (always !== undefined && late !== null) {
    const done = always.done.charAt(0).toUpperCase() + always.done.slice(1);
    return {
      square: "warn",
      text: `${done} in Settings, but the answer did not reach Vibe (${late}): ${summary}. ${always.after}`,
    };
  }
  switch (outcome.type) {
    case "answerNotConfirmed":
      return {
        square: "warn",
        text: "Not confirmed: Vibe did not confirm your answer.",
      };
    case "allowed":
      if (outcome.by === "grant")
        return { square: "ok", text: `Allowed for this session (your earlier answer): ${summary}` };
      if (always !== undefined)
        return {
          square: "ok",
          text: `Allowed, and ${always.done} in Settings: ${summary} · ${who}`,
        };
      return {
        square: "ok",
        text: `${outcome.scope === "session" ? "Allowed for this session" : "Allowed once"}: ${summary} · ${who}`,
      };
    case "refused": {
      if (outcome.by === "core")
        return {
          square: "warn",
          text: `Not allowed: ${summary}${outcome.note === null ? "" : ` (${outcome.note})`}`,
        };
      const note = notePart(outcome.note);
      return {
        square: "warn",
        text: `Said No: ${summary} · ${who}${note.text}`,
        ...(note.title === undefined ? {} : { title: note.title }),
      };
    }
    case "cancelled":
      return outcome.reason === "the turn was stopped"
        ? { square: "warn", text: `Stopped: ${summary}` }
        : { square: "warn", text: `Closed: ${summary} (${outcome.reason})` };
    case "never":
      return { square: "err", text: neverText(summary, record.ask.kind) };
  }
}

/** Where a Never is set, per kind. */
function neverCategory(kind: string): string {
  return kind === "vibe-work/fillLogin" || kind === "vibe-work/secretRun"
    ? "Project secrets"
    : "Safety";
}

/** The Never row: no card, a visible refusal. */
export function neverText(what: string, kind: string): string {
  return `Not allowed by your Never setting: ${what}. Change it in Settings › ${neverCategory(kind)}.`;
}
