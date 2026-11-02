/** Turn-time Vibe warnings become plain transcript notices at their arrival position.
 * Depends on session entries and configNotices; called by coreModel for notices, runtimes and snapshots.
 * Local notices survive refreshes, deduplicate by line and turn, and defer config problems to the bell.
 */
import type { JsonValue } from "../protocol/core";
import { isConfigProblem } from "./configNotices";
import type { SessionEntry } from "./sessionSummary";

type Obj = Record<string, JsonValue | undefined>;
const FALLBACK: Readonly<Record<string, string>> = {
  integrations_unavailable: "Some connectors or MCP servers are unavailable.",
  compaction_summary_cut_off: "Context summary was cut short.",
  compaction_inputs_reduced: "Less context was used for the summary.",
  request_too_long: "The request is too long.",
  images_replaced: "This model cannot see images.",
};

function obj(value: JsonValue | undefined): Obj | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
}
function list(value: JsonValue | undefined): JsonValue[] {
  return Array.isArray(value) ? value : [];
}
function line(value: JsonValue | undefined): string {
  return typeof value === "string" ? value.replace(/\s+/g, " ").trim() : "";
}
function local(value: JsonValue): boolean {
  return obj(value)?.["vwRuntimeWarning"] === true;
}

/** Fold one scoped warning into the session without authoring a Vibe history entry on the server. */
export function withRuntimeWarning(entry: SessionEntry, params: JsonValue): SessionEntry {
  if (entry.state === null) return entry;
  const warning = obj(obj(params)?.["warning"]);
  const raw = line(warning?.["message"]);
  const code = line(warning?.["code"]);
  // A successful lookup in the next catalogue source changes no user-visible capability.
  if (/^the model catalogue at .+ has no entry for .+; using the next source\.?$/i.test(raw))
    return entry;
  if (isConfigProblem(entry.runtime, raw)) return entry;
  const plain = raw !== "" && raw.length <= 180 && !/Traceback:|https?:\/\/|`|\/mcp\b/.test(raw);
  const message = plain ? raw : (FALLBACK[code] ?? "Vibe reported a warning.");
  const history = list(entry.state["history"]);
  const status = obj(entry.session["status"]);
  const turns = list(entry.state["turns"]);
  const turnId =
    obj(params)?.["turnId"] ?? status?.["activeTurnId"] ?? obj(turns.at(-1))?.["id"] ?? null;
  if (
    history.some((value) => {
      const notice = obj(value);
      return (
        notice?.["type"] === "notice" &&
        notice["turnId"] === turnId &&
        line(notice["message"]) === message
      );
    })
  )
    return entry;
  const last = obj(history.at(-1));
  const createdAt = typeof last?.["createdAt"] === "number" ? last["createdAt"] : 0;
  const notice = {
    id: `vw-warning:${encodeURIComponent(JSON.stringify([turnId, message]))}`,
    type: "notice",
    level: "warning",
    turnId,
    message,
    createdAt,
    vwRuntimeWarning: true,
    warningMessage: raw,
  } satisfies Obj;
  return { ...entry, state: { ...entry.state, history: [...history, notice] } };
}

/** Config problems stay in the bell even when their runtime arrives after their warning. */
export function withoutConfigWarnings(entry: SessionEntry): SessionEntry {
  if (entry.state === null) return entry;
  const history = list(entry.state["history"]);
  const next = history.filter(
    (value) =>
      !local(value) || !isConfigProblem(entry.runtime, line(obj(value)?.["warningMessage"])),
  );
  return next.length === history.length
    ? entry
    : { ...entry, state: { ...entry.state, history: next } };
}

/** Reinsert local notices after the nearest preceding entry still present in a fresh snapshot. */
export function keepRuntimeWarnings(state: Obj, previous: SessionEntry | undefined): Obj {
  const old = list(previous?.state?.["history"]);
  const notices = old.filter(local);
  if (notices.length === 0) return state;
  const history = [...list(state["history"])];
  for (const notice of notices) {
    const id = obj(notice)?.["id"];
    const warning = obj(notice);
    if (
      history.some((value) => {
        const current = obj(value);
        return (
          current?.["id"] === id ||
          (current?.["type"] === "notice" &&
            current["turnId"] === warning?.["turnId"] &&
            line(current["message"]) === line(warning?.["message"]))
        );
      })
    )
      continue;
    const preceding = old.slice(0, old.indexOf(notice)).map((value) => obj(value)?.["id"]);
    let at = -1;
    for (const previousId of preceding.reverse()) {
      at = history.findIndex((value) => obj(value)?.["id"] === previousId);
      if (at >= 0) break;
    }
    history.splice(at + 1, 0, notice);
  }
  return { ...state, history };
}
