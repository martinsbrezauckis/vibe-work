/**
 * The session's loops (Vibe's scheduled recurring prompts, protocol `loops/*`) as the Loops list shows them:
 * the rows read from a `loops/list` / `loops/create` / `loops/delete` reply, and the words for schedule, next run and
 * the notes `/loop` writes under the composer.
 * What a loop is in Vibe (`vibe/app_server/_unified_scheduled_loops.py`, 2.26.1): a prompt and an interval (at least
 * 30 s, whole seconds) or, when the model made it through its `cron` tool, a five-field cron expression. At most 50 per
 * session. A loop starts a normal turn of its session when it is due and the session is idle; a loop that comes due while
 * the session is busy waits. `nextFireAt` is epoch SECONDS (a float).
 * Depends on: protocol/core (`JsonValue`), transcript/model/entries (JSON helpers). Called by: state/composerLoops,
 * LoopsStrip, tests.
 * Invariants: pure (the clock comes in as an argument); a malformed loop in a reply is skipped, never thrown on; the
 * rows are ordered by next run, soonest first.
 */
import type { JsonValue } from "../../../protocol/core";
import { term } from "../../../vocabulary";
import { num, obj, str } from "../../transcript/model/entries";

/** One loop of a session. */
export interface LoopRow {
  id: string;
  prompt: string;
  /** The fixed interval in seconds, or `null` for a cron schedule. */
  everySeconds: number | null;
  /** The cron expression of a loop the model scheduled, or `null`. */
  cron: string | null;
  /** When it runs next, epoch seconds. */
  nextFireAt: number;
}

/** One loop from a reply, or `null` when it lacks an id or a time. */
export function loopRow(value: JsonValue | undefined): LoopRow | null {
  const o = obj(value);
  const id = str(o?.["id"]);
  const next = num(o?.["nextFireAt"]);
  if (o === null || id === null || id === "" || next === null) return null;
  return {
    id,
    prompt: str(o["prompt"]) ?? "",
    everySeconds: num(o["intervalSeconds"]),
    cron: str(o["cron"]),
    nextFireAt: next,
  };
}

/** The loops of a `loops/list` reply (`{loops: [...]}`), soonest first; empty for anything else. */
export function loopRows(reply: JsonValue | null | undefined): LoopRow[] {
  const list = obj(reply ?? null)?.["loops"];
  if (!Array.isArray(list)) return [];
  return list
    .map(loopRow)
    .filter((row): row is LoopRow => row !== null)
    .sort((a, b) => a.nextFireAt - b.nextFireAt);
}

/** The loop of a `loops/create` or `loops/delete` reply (`{loop: {...}}`), or `null`. */
export function replyLoop(reply: JsonValue | null | undefined): LoopRow | null {
  return loopRow(obj(reply ?? null)?.["loop"]);
}

/** "5m", "1h 30m", "45s": the two largest non-zero units of a span, never a negative one. */
export function formatSpan(seconds: number): string {
  let left = Math.max(0, Math.round(seconds));
  if (left === 0) return "0s";
  const parts: string[] = [];
  for (const [unit, size] of [
    ["d", 86_400],
    ["h", 3_600],
    ["m", 60],
    ["s", 1],
  ] as const) {
    const n = Math.floor(left / size);
    if (n > 0) {
      parts.push(`${String(n)}${unit}`);
      left -= n * size;
    }
  }
  return parts.slice(0, 2).join(" ");
}

/** How often a loop runs: "every 5m", "cron 0 9 * * 1-5" (a model-made schedule), or "on a schedule". */
export function everyText(row: LoopRow): string {
  if (row.everySeconds !== null) return `every ${formatSpan(row.everySeconds)}`;
  return row.cron === null ? "on a schedule" : `cron ${row.cron}`;
}

/** When a loop runs next, relative to `nowSeconds`: "in 4m 10s", or "due" once its time has passed (it waits for an idle session). */
export function nextText(row: LoopRow, nowSeconds: number): string {
  const left = row.nextFireAt - nowSeconds;
  return left > 0 ? `in ${formatSpan(Math.ceil(left))}` : "due";
}

/** The soonest loop, or `null` for none. */
export function soonest(rows: readonly LoopRow[]): LoopRow | null {
  return rows.reduce<LoopRow | null>(
    (best, row) => (best === null || row.nextFireAt < best.nextFireAt ? row : best),
    null,
  );
}

/** "3 loops": the count with its noun. */
export function loopCount(n: number): string {
  return `${String(n)} ${n === 1 ? "loop" : "loops"}`;
}

/** "1 loop runs by itself" / "3 loops run by themselves": the count as a sentence subject with its verb. */
export function loopsRun(n: number): string {
  return n === 1 ? "1 loop runs by itself" : `${String(n)} loops run by themselves`;
}

/** The prompt cut to `max` characters on one line, for notes and tooltips. */
export function shortPrompt(prompt: string, max = 60): string {
  const line = prompt.replace(/\s+/g, " ").trim();
  return line.length <= max ? line : `${line.slice(0, max - 1).trimEnd()}…`;
}

/**
 * The clause that says what a loop's turns may do: they run under the session's CURRENT access mode, like a message
 * the user typed. Full access gets a plain warning because nobody is there to approve anything.
 */
export function modeClause(modeId: string | null, label: string | null): string {
  if (modeId === "auto-approve")
    return `${label ?? term("accessModes", "auto-approve").name} is on: its steps run without asking you.`;
  return label === null
    ? "It uses this session's access mode."
    : `It uses this session's access mode (${label}).`;
}

/** The note written after `/loop <interval> <prompt>` made a loop. */
export function createdNote(row: LoopRow, nowSeconds: number, mode: string): string {
  return [
    `Loop ${row.id} added: "${shortPrompt(row.prompt)}" ${everyText(row)}, next run ${nextText(row, nowSeconds)}.`,
    "It starts turns by itself while this session is open and idle.",
    mode,
    `Stop it with /loop cancel ${row.id} or Delete in the Loops list.`,
  ].join(" ");
}

/** The note for `/loop list` (also written when the list is empty). */
export function listedNote(count: number): string {
  return count === 0
    ? "No loops in this session. Add one: /loop 5m <what to repeat>."
    : `${loopCount(count)} in this session: see the Loops list above the message box.`;
}
