/**
 * The transcript model: one session's history entries and turns -> the flat list of rows the view
 * renders. Pure functions, no React.
 *
 * Shape of a turn ("Steps grouping"):
 *   user message -> one collapsed steps group (effects, reasoning, resolved callbacks and the assistant
 *   narration that more steps follow, in order) -> the assistant answer (messages after the last step)
 *   -> the end-of-turn row (complete / failed / interrupted).
 * An open callback is never hidden in a collapsed group: it is its own card row where it occurs and
 * becomes an audit item of the group once Vibe resolves it. A steer (a second user message in a turn)
 * starts a new segment. Notices are pills and checkpoints are dividers, both rows of their own.
 * Entries without a `turnId` join the turn whose time window holds their `createdAt`; otherwise they
 * are session-level rows in history order.
 *
 * Depends on: entries.ts. Called by: memo.ts (`createTranscriptBuilder`), the golden test.
 * Invariants: every conversational entry with an id lands in one row (`rowEntries` lists them);
 * hook lifecycle/no-op events remain only in raw history for diagnostics; an id seen
 * twice (Vibe can re-announce an entry) is one row with the latest content at the first position;
 * unknown entry types become `other` rows and unknown kinds generic steps (never a throw); a history
 * without effects (a unified fork) is just messages.
 */
import type { JsonValue } from "../../../protocol/core";
import {
  type Obj,
  createdAt,
  effectView,
  entryId,
  entryType,
  inProgress,
  num,
  obj,
  roleOf,
  statusOf,
  str,
  turnIdOf,
} from "./entries";
import { isWebSearch } from "./sources";
import { hookLine, isHookNotice, transcriptEntries } from "./hookNotices";

/** The `searches` of an answer without web searches (one shared list: rows stay comparable). */
const NO_SEARCHES: readonly Obj[] = Object.freeze([]);

/** One item inside a steps group. */
export type StepItem =
  | { kind: "effect"; entry: Obj }
  | { kind: "reasoning"; entry: Obj }
  | { kind: "narration"; entry: Obj }
  /** A resolved callback; `related` is the effect it gated, when it is in the history. */
  | { kind: "audit"; entry: Obj; related: Obj | null };

/** One rendered row. `key` is stable across updates of the same logical row. */
export type Row =
  | { kind: "user"; key: string; entry: Obj; steer: boolean }
  /**
   * `searches`: the web search effects of the steps before this answer in the same part of the turn
   * (its "Sources (n)" chip); the shared empty list when there were none.
   */
  | { kind: "assistant"; key: string; entry: Obj; searches: readonly Obj[] }
  | {
      kind: "steps";
      key: string;
      items: readonly StepItem[];
      /** Something in the group is still happening (or the turn is still working on it). */
      live: boolean;
      /** Vibe's `statusText` of the newest live effect ("" when none). */
      statusText: string;
      /** Number of effect items. */
      effects: number;
      /** Effect items that failed. */
      failed: number;
    }
  | {
      kind: "callback";
      key: string;
      entry: Obj;
      related: Obj | null;
      /**: a subagent's card shown in its parent's transcript: the child session that answers it. */
      answerFor?: string;
    }
  | { kind: "protectedChange"; key: string; entry: Obj }
  /** A Chrome Control card, or its audit row (a `chrome_ask` entry the core weaves into the history). */
  | { kind: "chromeAsk"; key: string; entry: Obj }
  | { kind: "notice"; key: string; entry: Obj }
  | { kind: "hook"; key: string; entry: Obj; text: string; error: boolean }
  | { kind: "divider"; key: string; entry: Obj }
  | { kind: "turnEnd"; key: string; turn: Obj }
  | { kind: "other"; key: string; entry: Obj };

/** Turn statuses that end a turn with an end row (`PublicTurnStatus`). */
const ENDED = new Set(["completed", "failed", "interrupted"]);

/** True for a callback Vibe still waits on. */
function callbackOpen(entry: Obj): boolean {
  const status = statusOf(entry);
  return status === "open" || (status === "" && inProgress(entry));
}

function isStep(entry: Obj): boolean {
  const type = entryType(entry);
  return type === "effect" || type === "reasoning" || (type === "callback" && !callbackOpen(entry));
}

/** Keeps the first position of every id with the latest content; drops entries without an id. */
function dedupe(history: readonly JsonValue[]): Obj[] {
  const out: Obj[] = [];
  const at = new Map<string, number>();
  for (const raw of history) {
    const entry = obj(raw);
    if (entry === null) continue;
    const id = entryId(entry);
    if (id === "") continue;
    const index = at.get(id);
    if (index === undefined) {
      at.set(id, out.length);
      out.push(entry);
    } else out[index] = entry;
  }
  return out;
}

interface TurnInfo {
  turn: Obj;
  startedAt: number;
  completedAt: number | null;
}

function turnTable(turns: readonly JsonValue[]): Map<string, TurnInfo> {
  const table = new Map<string, TurnInfo>();
  for (const raw of turns) {
    const turn = obj(raw);
    const id = turn === null ? null : str(turn["id"]);
    if (turn === null || id === null) continue;
    table.set(id, {
      turn,
      startedAt: num(turn["startedAt"]) ?? 0,
      completedAt: num(turn["completedAt"]),
    });
  }
  return table;
}

/** The turn whose time window holds `at` (the latest started one wins), or `null`. */
function turnAt(table: Map<string, TurnInfo>, at: number): string | null {
  let best: string | null = null;
  let bestStart = -Infinity;
  for (const [id, info] of table) {
    const inside =
      info.startedAt > 0 &&
      info.startedAt <= at &&
      (info.completedAt === null || at <= info.completedAt);
    if (inside && info.startedAt > bestStart) {
      best = id;
      bestStart = info.startedAt;
    }
  }
  return best;
}

type Block = { turnId: string | null; entries: Obj[] };

/** Splits the entries into turn blocks (at their first entry) and runs of session-level entries. */
function blocksOf(entries: readonly Obj[], table: Map<string, TurnInfo>): Block[] {
  const blocks: Block[] = [];
  const byTurn = new Map<string, Block>();
  let loose: Block | null = null;
  for (const entry of entries) {
    // A turnless entry inside a turn's time window (a shell run, an automatic compaction during the
    // turn) shows where it happened; between turns it is a session-level row.
    const turnId = turnIdOf(entry) ?? turnAt(table, createdAt(entry));
    if (turnId === null) {
      if (loose === null) {
        loose = { turnId: null, entries: [] };
        blocks.push(loose);
      }
      loose.entries.push(entry);
      continue;
    }
    loose = null;
    let block = byTurn.get(turnId);
    if (block === undefined) {
      block = { turnId, entries: [] };
      byTurn.set(turnId, block);
      blocks.push(block);
    }
    block.entries.push(entry);
  }
  return blocks;
}

function stepsRow(items: StepItem[], forceLive: boolean): Row {
  let live = forceLive;
  let statusText = "";
  let effects = 0;
  let failed = 0;
  for (const item of items) {
    if (item.kind === "effect") {
      effects += 1;
      const view = effectView(item.entry);
      if (view.status === "failed") failed += 1;
      if (view.live) {
        live = true;
        if (view.statusText !== "") statusText = view.statusText;
      }
    } else if (item.kind !== "audit" && inProgress(item.entry)) live = true;
  }
  const first = items[0];
  return {
    kind: "steps",
    key: `steps:${first === undefined ? "" : entryId(first.entry)}`,
    items,
    live,
    statusText,
    effects,
    failed,
  };
}

/** Rows of one segment: the entries after a user message (or a turn's start) up to the next one. */
function segmentRows(segment: readonly Obj[], byId: Map<string, Obj>, rows: Row[]): void {
  let lastStep = -1;
  segment.forEach((entry, i) => {
    if (isStep(entry)) lastStep = i;
  });
  let group: StepItem[] = [];
  const searches: Obj[] = [];
  const flush = (): void => {
    if (group.length > 0) rows.push(stepsRow(group, false));
    group = [];
  };
  const related = (entry: Obj): Obj | null => {
    const id = str(entry["relatedEntryId"]);
    return id === null ? null : (byId.get(id) ?? null);
  };
  segment.forEach((entry, i) => {
    const id = entryId(entry);
    switch (entryType(entry)) {
      case "protected_change":
        flush();
        rows.push({ kind: "protectedChange", key: `protected:${id}`, entry });
        return;
      case "chrome_ask":
        flush();
        rows.push({ kind: "chromeAsk", key: id, entry });
        return;
      case "effect":
        group.push({ kind: "effect", entry });
        if (isWebSearch(entry)) searches.push(entry);
        return;
      case "reasoning":
        group.push({ kind: "reasoning", entry });
        return;
      case "callback":
        if (callbackOpen(entry)) {
          flush();
          rows.push({ kind: "callback", key: `cb:${id}`, entry, related: related(entry) });
        } else group.push({ kind: "audit", entry, related: related(entry) });
        return;
      case "message":
        if (roleOf(entry) !== "assistant") break;
        if (i < lastStep) group.push({ kind: "narration", entry });
        else {
          flush();
          rows.push({
            kind: "assistant",
            key: `msg:${id}`,
            entry,
            searches: searches.length === 0 ? NO_SEARCHES : [...searches],
          });
        }
        return;
      case "notice":
        flush();
        if (isHookNotice(entry)) {
          const line = hookLine(entry, byId);
          if (line !== null) rows.push({ kind: "hook", key: `hook:${id}`, entry, ...line });
          return;
        }
        rows.push({ kind: "notice", key: `notice:${id}`, entry });
        return;
      case "checkpoint":
        flush();
        rows.push({ kind: "divider", key: `cp:${id}`, entry });
        return;
      default:
        break;
    }
    flush();
    rows.push({ kind: "other", key: `other:${id}`, entry });
  });
  flush();
}

function blockRows(block: Block, info: TurnInfo | undefined, byId: Map<string, Obj>): Row[] {
  const rows: Row[] = [];
  let segment: Obj[] = [];
  let users = 0;
  for (const entry of block.entries) {
    if (roleOf(entry) === "user") {
      segmentRows(segment, byId, rows);
      segment = [];
      const steer = users > 0 || entry["source"] === "turn_steer";
      users += 1;
      rows.push({ kind: "user", key: `user:${entryId(entry)}`, entry, steer });
    } else segment.push(entry);
  }
  segmentRows(segment, byId, rows);
  if (info === undefined) return rows;
  const status = str(info.turn["status"]) ?? "";
  const last = rows[rows.length - 1];
  // While the turn runs and its newest row is the steps group, the model is working on it (thinking
  // between steps): the group stays live.
  if (status === "in_progress" && last?.kind === "steps" && !last.live) {
    rows[rows.length - 1] = { ...last, live: true };
  }
  if (ENDED.has(status))
    rows.push({ kind: "turnEnd", key: `end:${block.turnId ?? ""}`, turn: info.turn });
  return rows;
}

/**
 * Builds the rows for a session's `history` and `turns` (the `PublicSessionState` fields).
 * Never throws on unexpected shapes; a value that is not an entry object is skipped.
 */
export function buildRows(history: readonly JsonValue[], turns: readonly JsonValue[]): Row[] {
  const entries = dedupe(history);
  const table = turnTable(turns);
  const settled = entries.map((entry) => {
    if (
      entryType(entry) !== "checkpoint" ||
      entry["kind"] !== "compaction" ||
      entry["generationStatus"] !== "in_progress"
    )
      return entry;
    const turn = table.get(str(entry["turnId"]) ?? "");
    if (turn === undefined || !ENDED.has(str(turn.turn["status"]) ?? "")) return entry;
    return { ...entry, generationStatus: "completed", message: "Context compaction interrupted" };
  });
  const byId = new Map(settled.map((entry) => [entryId(entry), entry]));
  return blocksOf(transcriptEntries(settled), table).flatMap((block) =>
    blockRows(block, block.turnId === null ? undefined : table.get(block.turnId), byId),
  );
}

/** The history entries a row shows (the audit's related effect is shown by its own row). */
export function rowEntries(row: Row): Obj[] {
  switch (row.kind) {
    case "steps":
      return row.items.map((item) => item.entry);
    case "turnEnd":
      return [];
    default:
      return [row.entry];
  }
}
