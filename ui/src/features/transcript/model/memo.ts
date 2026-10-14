/**
 * Row identity across rebuilds: a streamed patch to one entry must re-render that entry's row only
 * ("memoised per entry id + updatedAt so a streaming patch re-renders one row only").
 *
 * The session store keeps every unpatched entry by reference (`state/sessionChanges.ts` clones only
 * the patched entry), so reference identity is the exact "did this entry change" test: a patch,
 * an upsert or a fresh snapshot always produces a new object, and an untouched entry never does.
 * (`id` + `updatedAt` alone would miss a patch that does not bump `updatedAt`.) A rebuilt row whose
 * referenced entries and flags are all identical returns the previous row object, so `memo` row
 * components skip it.
 * Depends on: build.ts. Called by: the transcript view (one builder per mounted transcript), tests.
 * Invariants: pure apart from the cache; the cache only holds the previous build's rows.
 */
import type { JsonValue } from "../../../protocol/core";
import { type Row, type StepItem, buildRows } from "./build";

function sameItems(a: readonly StepItem[], b: readonly StepItem[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((item, i) => {
    const other = b[i];
    if (other === undefined || other.kind !== item.kind || other.entry !== item.entry) return false;
    return item.kind !== "audit" || (other.kind === "audit" && other.related === item.related);
  });
}

/** True when `next` would render exactly like `prev`. */
export function sameRow(prev: Row, next: Row): boolean {
  if (prev.kind !== next.kind || prev.key !== next.key) return false;
  switch (next.kind) {
    case "steps":
      return (
        prev.kind === "steps" &&
        prev.live === next.live &&
        prev.statusText === next.statusText &&
        prev.effects === next.effects &&
        prev.failed === next.failed &&
        sameItems(prev.items, next.items)
      );
    case "turnEnd":
      return prev.kind === "turnEnd" && prev.turn === next.turn;
    case "user":
      return prev.kind === "user" && prev.entry === next.entry && prev.steer === next.steer;
    case "callback":
      return prev.kind === "callback" && prev.entry === next.entry && prev.related === next.related;
    case "assistant":
      return (
        prev.kind === "assistant" &&
        prev.entry === next.entry &&
        prev.searches.length === next.searches.length &&
        prev.searches.every((entry, i) => entry === next.searches[i])
      );
    case "hook":
      return (
        prev.kind === "hook" &&
        prev.entry === next.entry &&
        prev.text === next.text &&
        prev.error === next.error
      );
    default:
      return "entry" in prev && prev.entry === next.entry;
  }
}

/** A builder that remembers its last output. */
export interface TranscriptBuilder {
  /** Rows for this state; unchanged rows are the previous objects. */
  build(history: readonly JsonValue[], turns: readonly JsonValue[]): readonly Row[];
}

/** Creates a builder (one per transcript view). */
export function createTranscriptBuilder(): TranscriptBuilder {
  let lastHistory: readonly JsonValue[] | null = null;
  let lastTurns: readonly JsonValue[] | null = null;
  let lastRows: readonly Row[] = [];
  let byKey = new Map<string, Row>();
  return {
    build(history, turns) {
      if (history === lastHistory && turns === lastTurns) return lastRows;
      const fresh = buildRows(history, turns);
      const rows = fresh.map((row) => {
        const prev = byKey.get(row.key);
        return prev !== undefined && sameRow(prev, row) ? prev : row;
      });
      const unchanged =
        rows.length === lastRows.length && rows.every((row, i) => row === lastRows[i]);
      lastHistory = history;
      lastTurns = turns;
      if (!unchanged) {
        lastRows = rows;
        byKey = new Map(rows.map((row) => [row.key, row]));
      }
      return lastRows;
    },
  };
}
