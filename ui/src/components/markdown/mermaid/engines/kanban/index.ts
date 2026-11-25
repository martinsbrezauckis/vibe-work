import type { DiagramEngine } from "../../types";
import { drawKanban } from "./draw";
import { parseKanban } from "./parse";

/** The engine registered in groups/timelines.ts: parse the prepared source, then draw it. */
export const engine: DiagramEngine = {
  render: (prepared) => drawKanban(parseKanban(prepared)),
};
