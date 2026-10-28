/**
 * Bounded metadata trace of commands sent by the UI. Depends on typed commands; createApp records at dispatch
 * and exposes reads through uiControl. Dispatch copies fixed-size metadata; formatting happens only on read.
 * Raw command input is never retained, serialized or walked recursively.
 */
import type { Command, CoreError } from "../protocol/core";

/** Maximum retained commands, independently of the size of their input. */
export const TRACE_CAPACITY = 500;
const META_FIELDS = [
  "id",
  "sessionId",
  "callbackId",
  "projectId",
  "libraryId",
  "docId",
  "tabId",
  "terminalId",
  "name",
];

function object(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}
function short(value: unknown): string | null {
  return typeof value === "string" ? value.slice(0, 64).replace(/[\r\n\t]/g, " ") : null;
}

/** Dispatch completion metadata; errors retain only the structured code. */
export type TraceOutcome =
  { status: "pending" } | { status: "succeeded" } | { status: "failed"; code: CoreError["code"] };

/** One attempted dispatch, including commands the core refused. */
interface TraceEntry {
  seq: number;
  time: string;
  type: Command["type"];
  op: string | null;
  method: string | null;
  summary: string;
  testId: string | null;
  outcome: TraceOutcome;
}

type CapturedEntry = Omit<TraceEntry, "time" | "summary"> & { time: number; fields: string[] };

/** Creates a ring that retains only short metadata. Reading it never calls the core. */
export function createCommandTrace() {
  const ring = new Array<CapturedEntry | undefined>(TRACE_CAPACITY);
  let seq = 0;
  return {
    record(command: Command, testId: string | null) {
      const top = object(command);
      const request = object(top["request"]);
      const fields: string[] = [];
      for (const source of [top, request, object(top["params"])]) {
        for (const key of META_FIELDS) {
          const value = short(source[key]);
          if (value !== null && fields.length < 5) fields.push(`${key}=${value}`);
        }
      }
      const entry: CapturedEntry = {
        seq: ++seq,
        outcome: { status: "pending" },
        time: Date.now(),
        type: command.type,
        op: short(request["op"]),
        method: short(top["method"] ?? request["method"]),
        fields,
        testId: testId?.slice(0, 128) ?? null,
      };
      ring[(seq - 1) % TRACE_CAPACITY] = entry;
      return (outcome: Exclude<TraceOutcome, { status: "pending" }>): void => {
        entry.outcome = outcome;
      };
    },
    read(since = 0) {
      const oldestSeq = seq === 0 ? null : Math.max(1, seq - TRACE_CAPACITY + 1);
      const entries: TraceEntry[] = [];
      for (let n = Math.max(since + 1, oldestSeq ?? 1); n <= seq; n++) {
        const entry = ring[(n - 1) % TRACE_CAPACITY];
        if (entry) {
          const { fields, time, ...metadata } = entry;
          entries.push({
            ...metadata,
            time: new Date(time).toISOString(),
            summary: fields.join(", ").slice(0, 200),
          });
        }
      }
      return {
        entries,
        latestSeq: seq,
        oldestSeq,
        truncated: oldestSeq !== null && since < oldestSeq - 1,
      };
    },
  };
}
