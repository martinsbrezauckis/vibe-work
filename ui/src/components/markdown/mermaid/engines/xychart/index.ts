import type { DiagramEngine } from "../../types";
import { drawXyChart } from "./draw";
import { parseXyChart } from "./parse";

/** The XY chart engine registered in groups/charts.ts. */
export const engine: DiagramEngine = {
  render: (prepared) => drawXyChart(parseXyChart(prepared)),
};
