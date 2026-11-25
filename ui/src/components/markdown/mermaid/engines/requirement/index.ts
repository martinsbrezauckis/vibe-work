import type { DiagramEngine } from "../../types";
import { drawRequirement } from "./draw";
import { parseRequirement } from "./parse";

/** The requirement engine registered in groups/architecture.ts: parse prepared lines and draw SVG. */
export const engine: DiagramEngine = {
  render: (prepared) => drawRequirement(parseRequirement(prepared)),
};
