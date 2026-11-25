import type { DiagramEngine } from "../../types";
import { drawClass } from "./draw";
import { parseClass } from "./parse";

/** The engine registered in groups/core.ts: parse the prepared source, then draw it. */
export const engine: DiagramEngine = {
  render: (prepared) => drawClass(parseClass(prepared)),
};
