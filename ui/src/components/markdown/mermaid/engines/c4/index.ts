import type { DiagramEngine } from "../../types";
import { drawC4 } from "./draw";
import { parseC4 } from "./parse";

/** The C4 engine registered in groups/architecture.ts: parse prepared lines and draw SVG. */
export const engine: DiagramEngine = {
  render: (prepared) => drawC4(parseC4(prepared)),
};
