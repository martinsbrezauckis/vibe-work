import type { DiagramEngine } from "../../types";
import { drawSankey } from "./draw";
import { parseSankey } from "./parse";

/** The Sankey diagram engine registered in groups/charts.ts. */
export const engine: DiagramEngine = {
  render: (prepared) => drawSankey(parseSankey(prepared)),
};
