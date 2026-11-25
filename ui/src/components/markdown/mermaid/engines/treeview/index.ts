import type { DiagramEngine } from "../../types";
import { drawTreeview } from "./draw";
import { parseTreeview } from "./parse";

/** The TreeView diagram engine registered in groups/hierarchy.ts. */
export const engine: DiagramEngine = {
  render: (prepared) => drawTreeview(parseTreeview(prepared)),
};
