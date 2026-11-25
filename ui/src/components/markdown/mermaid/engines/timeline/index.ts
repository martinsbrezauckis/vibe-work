import type { DiagramEngine } from "../../types";
import { drawTimeline } from "./draw";
import { parseTimeline } from "./parse";

/** The engine registered in groups/timelines.ts: parse the prepared source, then draw it. */
export const engine: DiagramEngine = {
  render: (prepared) => drawTimeline(parseTimeline(prepared)),
};
