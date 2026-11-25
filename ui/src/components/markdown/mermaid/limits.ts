/** Hard ceilings every engine enforces; each breach gives a readable "Diagram too large" error naming the bound. */
import { DiagramLimitError } from "./types";

/** Longest source accepted, in characters. */
export const MAX_SOURCE_CHARS = 8000;
/** Longest physical source line, checked before comments, trimming, or engine loading. */
export const MAX_LINE_CHARS = 1000;
/** Participants, classes, states, entities, slices, tasks, blocks... */
export const MAX_NODES = 120;
/** Messages, relations, transitions, dependencies... */
export const MAX_EDGES = 240;
/** Nested blocks, composite states, tree depth. */
export const MAX_DEPTH = 16;
/** Members per class, attributes per entity, statements in one block. */
export const MAX_MEMBERS = 60;
/** Cap on a drawn label; longer text is cut with an ellipsis, never silently dropped. */
export const MAX_LABEL_CHARS = 240;
/** Layout and drawing step budget shared by every loop that is not a plain line scan. */
export const MAX_STEPS = 400_000;
/** Largest drawing side in user units. */
export const MAX_SVG_SIDE = 16_000;
// There is deliberately no line-count bound: 8000 characters already bound the lines, and an
// ordinary gantt or sequence diagram that fits in 8000 characters must never be refused for its lines.

/** Refuse `count` above `max` naming what was counted. */
export function checkCount(count: number, max: number, what: string): void {
  if (count > max)
    throw new DiagramLimitError(`Diagram too large: more than ${String(max)} ${what}`);
}

/** Counts steps of layout loops; a hostile input cannot spin the page. */
export class StepBudget {
  private used = 0;
  constructor(private readonly max = MAX_STEPS) {}
  tick(amount = 1): void {
    this.used += amount;
    if (this.used > this.max)
      throw new DiagramLimitError("Diagram too large: too many layout steps");
  }
}

/** Cap a drawing side so a viewBox can never be huge. */
export function checkSide(side: number): number {
  if (!Number.isFinite(side) || side > MAX_SVG_SIDE) {
    throw new DiagramLimitError(
      `Diagram too large: wider or taller than ${String(MAX_SVG_SIDE)} units`,
    );
  }
  return Math.max(side, 1);
}
