import type { DiagramEngine } from "../../types";
import { drawQuadrant } from "./draw";
import { parseQuadrant } from "./parse";

/** The quadrant chart engine registered in groups/charts.ts. */
export const engine: DiagramEngine = {
  render: (prepared) => drawQuadrant(parseQuadrant(prepared)),
};
