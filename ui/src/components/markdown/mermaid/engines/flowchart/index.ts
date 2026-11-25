/** Flowchart / graph diagram engine entry point registered in groups/core.ts. */
import type { DiagramEngine } from "../../types";
import { drawFlowchart } from "./draw";
import { parseFlowchart } from "./parse";

/** The flowchart engine registered in groups/core.ts. */
export const engine: DiagramEngine = {
  render: (prepared) => drawFlowchart(parseFlowchart(prepared)),
};
