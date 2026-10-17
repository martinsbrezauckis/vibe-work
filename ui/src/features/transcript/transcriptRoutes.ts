/**
 * The `ui/transcript` UI-control route. The Rust side lists the same name
 * (`crates/vw-host/src/host.rs` SHELL_ROUTES). Actions go through the same functions a click uses, so
 * the route can never do something the UI cannot:
 * - `read` (default): the rows of a session as rendered (kinds, keys, entry ids, card state);
 * - `open {key, open?}`: opens or closes a group, step or "show all" like a click;
 * - `answer {callbackId, decision | answers | cancel, scope?, feedback?}`: answers a card exactly as its
 *   buttons do (the answer is built by the same model functions and marked "via UI control" in the
 *   audit row);
 * - `link {url}`: shows the link dialog for a URL (what a click on a link in a message does);
 * - `sources {key, open?}`: opens or closes an answer's "Sources (n)" side list like its
 *   chip; `read` lists each answer's sources and each web search step's source count.
 * Depends on: AppContext types, the transcript model, the stores. Called by: useTranscriptWiring.
 * Invariants: bodies are validated (`invalidParams`); `sessionId` defaults to the selected session;
 * an unknown callback is `notFound`.
 */
import type { JsonValue } from "../../protocol/core";
import { UiControlError } from "../../state/uiControl";
import type { TranscriptUi } from "../../state/transcriptStore";
import type { App } from "../shell/AppContext";
import { type Row, rowEntries } from "./model/build";
import { createTranscriptBuilder } from "./model/memo";
import { subagentRows } from "./model/subagentRows";
import {
  DECISIONS,
  type Decision,
  type Picked,
  approvalOutput,
  offeredDecisions,
  questionCancelOutput,
  questionOutput,
  questionsOf,
} from "./model/callbacks";
import { type Obj, detailOf, entryId, messageText, obj, statusOf, str } from "./model/entries";
import { gatherSources, isWebSearch, searchSources } from "./model/sources";

function asObject(body: unknown): Record<string, unknown> {
  if (typeof body === "object" && body !== null && !Array.isArray(body))
    return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function text(value: unknown, name: string): string {
  if (typeof value === "string" && value !== "") return value;
  throw new UiControlError("invalidParams", `${name} must be a non-empty string`);
}

function list(value: JsonValue | undefined): readonly JsonValue[] {
  return Array.isArray(value) ? value : [];
}

function disclosure(key: string, choices: TranscriptUi["open"], defaultOpen: boolean): boolean {
  return choices[key] ?? defaultOpen;
}

function rowJson(row: Row, choices: TranscriptUi["open"], expanded: boolean): unknown {
  const base = { kind: row.kind, key: row.key, entryIds: rowEntries(row).map(entryId) };
  switch (row.kind) {
    case "user":
      return { ...base, text: messageText(row.entry).slice(0, 2000), steer: row.steer };
    case "assistant":
      return {
        ...base,
        text: messageText(row.entry).slice(0, 2000),
        steer: false,
        sources: gatherSources(row.searches),
      };
    case "steps":
      return {
        ...base,
        open: disclosure(
          row.key,
          choices,
          expanded && row.items.some((item) => item.kind === "reasoning"),
        ),
        live: row.live,
        statusText: row.statusText,
        effects: row.effects,
        failed: row.failed,
        items: row.items.map((item) => ({
          kind: item.kind,
          id: entryId(item.entry),
          status: statusOf(item.entry),
          ...(item.kind === "reasoning"
            ? { open: disclosure(`think:${entryId(item.entry)}`, choices, expanded) }
            : {}),
          ...(isWebSearch(item.entry) ? { sources: searchSources(item.entry).length } : {}),
        })),
      };
    case "callback":
      return {
        ...base,
        callbackId: str(row.entry["callbackId"]),
        ...(row.answerFor === undefined ? {} : { answerFor: row.answerFor }),
        callbackKind: str(detailOf(row.entry)["kind"]),
        status: statusOf(row.entry),
        choices: offeredDecisions(row.entry),
      };
    case "turnEnd":
      return { ...base, status: str(row.turn["status"]), turnId: str(row.turn["id"]) };
    case "hook":
      return { ...base, text: row.text, error: row.error };
    default:
      return { ...base, text: str(row.entry["message"]) ?? "" };
  }
}

/** Registers the route; returns the function that removes it. */
export function registerTranscriptRoutes(app: App): () => void {
  const sessionOf = (input: Record<string, unknown>): string => {
    const id = input["sessionId"] ?? app.shell.store.get().selected;
    return text(id, "sessionId");
  };
  const stateOf = (sessionId: string) => {
    const entry = app.core?.store.get().sessions[sessionId];
    if (entry === undefined)
      throw new UiControlError("notFound", `no session ${sessionId} in the UI`);
    return entry;
  };
  const rowsOf = (sessionId: string) => {
    const entry = stateOf(sessionId);
    const built = createTranscriptBuilder().build(
      list(entry.state?.["history"]),
      list(entry.state?.["turns"]),
    );
    const children = subagentRows(
      app.core?.store.get().subagentCards ?? {},
      sessionId,
      entry.state?.["childSessions"],
    );
    return [...built, ...children];
  };
  const read = (sessionId: string) => {
    const entry = stateOf(sessionId);
    const rows = rowsOf(sessionId);
    const choices = app.transcript.store.get().open;
    const expanded = app.shell.store.get().showThinkingExpanded;
    return {
      sessionId,
      loaded: entry.state !== null,
      openCallbackIds: [
        ...new Set([
          ...entry.openCallbackIds,
          ...rows.flatMap((row) =>
            row.kind === "callback" &&
            row.answerFor !== undefined &&
            str(row.entry["callbackId"]) !== null
              ? [str(row.entry["callbackId"])]
              : [],
          ),
        ]),
      ],
      answered: app.transcript.store.get().answered,
      problems: app.transcript.store.get().problems,
      rows: rows.map((row) => rowJson(row, choices, expanded)),
    };
  };
  const callbackEntry = (
    sessionId: string,
    callbackId: string,
  ): { entry: Obj; answerFor: string } => {
    for (const raw of list(stateOf(sessionId).state?.["history"])) {
      const entry = obj(raw);
      if (entry !== null && entry["type"] === "callback" && entry["callbackId"] === callbackId)
        return { entry, answerFor: sessionId };
    }
    const child = rowsOf(sessionId).find(
      (row) =>
        row.kind === "callback" &&
        row.answerFor !== undefined &&
        row.entry["callbackId"] === callbackId,
    );
    if (child?.kind === "callback" && child.answerFor !== undefined)
      return { entry: child.entry, answerFor: child.answerFor };
    throw new UiControlError("notFound", `no callback ${callbackId} in this session's history`);
  };
  const answer = async (input: Record<string, unknown>) => {
    const sessionId = sessionOf(input);
    const callbackId = text(input["callbackId"], "callbackId");
    const { entry, answerFor } = callbackEntry(sessionId, callbackId);
    let output: JsonValue | null;
    if (detailOf(entry)["kind"] === "user_input") {
      const { questions } = questionsOf(entry);
      const raw = Array.isArray(input["answers"]) ? (input["answers"] as unknown[]) : [];
      const picks: Picked[] = raw.map((a) => {
        const o = typeof a === "object" && a !== null ? (a as Record<string, unknown>) : {};
        const labels = Array.isArray(o["labels"])
          ? o["labels"].filter((l): l is string => typeof l === "string")
          : [];
        return { labels, other: typeof o["other"] === "string" ? o["other"] : "" };
      });
      output = input["cancel"] === true ? questionCancelOutput() : questionOutput(questions, picks);
      if (output === null)
        throw new UiControlError("invalidParams", "every question needs an answer");
    } else {
      const decision = input["decision"];
      if (!DECISIONS.includes(decision as Decision)) {
        throw new UiControlError(
          "invalidParams",
          `decision must be one of ${DECISIONS.join(", ")}`,
        );
      }
      output = approvalOutput(
        decision as Decision,
        typeof input["scope"] === "string" ? input["scope"] : undefined,
        typeof input["feedback"] === "string" ? input["feedback"] : undefined,
      );
    }
    const ok = await app.transcript.answer(answerFor, callbackId, output, "uiControl");
    return {
      ok,
      problem: app.transcript.store.get().problems[callbackId] ?? null,
      ...read(sessionId),
    };
  };

  const showSources = (input: Record<string, unknown>) => {
    const sessionId = sessionOf(input);
    const key = text(input["key"], "key");
    if (input["open"] === false) {
      app.transcript.showSources(null);
      return { sources: null };
    }
    const entry = stateOf(sessionId);
    const rows = createTranscriptBuilder().build(
      list(entry.state?.["history"]),
      list(entry.state?.["turns"]),
    );
    const row = rows.find((r) => r.key === key);
    const items = row?.kind === "assistant" ? gatherSources(row.searches) : [];
    if (items.length === 0)
      throw new UiControlError("notFound", `no answer with sources at ${key}`);
    app.transcript.showSources({ key, items });
    return { sources: app.transcript.store.get().sources };
  };

  return app.uiControl.register("transcript", async (body) => {
    const input = asObject(body);
    const action = input["action"] ?? "read";
    switch (action) {
      case "read":
        return read(sessionOf(input));
      case "open":
        {
          const key = text(input["key"], "key");
          const chosen = input["open"];
          const session = stateOf(sessionOf(input));
          const rows = createTranscriptBuilder().build(
            list(session.state?.["history"]),
            list(session.state?.["turns"]),
          );
          const steps = rows.find((row) => row.kind === "steps" && row.key === key);
          const expanded = app.shell.store.get().showThinkingExpanded;
          const defaultOpen =
            expanded &&
            (key.startsWith("think:") ||
              (steps?.kind === "steps" && steps.items.some((item) => item.kind === "reasoning")));
          const current = disclosure(key, app.transcript.store.get().open, defaultOpen);
          app.transcript.toggle(key, typeof chosen === "boolean" ? chosen : !current);
        }
        return read(sessionOf(input));
      case "answer":
        return answer(input);
      case "link":
        app.transcript.askLink(text(input["url"], "url"));
        return { link: app.transcript.store.get().link };
      case "sources":
        return showSources(input);
      default:
        throw new UiControlError(
          "invalidParams",
          "action must be read, open, answer, link or sources",
        );
    }
  });
}
