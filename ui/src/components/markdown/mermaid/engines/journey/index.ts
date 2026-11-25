import type { DiagramEngine } from "../../types";
import { drawJourney } from "./draw";
import { parseJourney } from "./parse";

/** The engine registered in groups/timelines.ts: parse the prepared source, then draw it. */
export const engine: DiagramEngine = {
  render: (prepared) => drawJourney(parseJourney(prepared)),
};
