import type { DiagramEngine } from "../../types";
import { drawRadar } from "./draw";
import { parseRadar } from "./parse";

/** The radar chart engine registered in groups/charts.ts. */
export const engine: DiagramEngine = {
  render: (prepared) => drawRadar(parseRadar(prepared)),
};
