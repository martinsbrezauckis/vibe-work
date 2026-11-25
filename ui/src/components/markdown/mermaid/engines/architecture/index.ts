import type { DiagramEngine } from "../../types";
import { drawArchitecture } from "./draw";
import { parseArchitecture } from "./parse";

/** The architecture engine registered in groups/architecture.ts: parse prepared lines and draw SVG. */
export const engine: DiagramEngine = {
  render: (prepared) => drawArchitecture(parseArchitecture(prepared)),
};
