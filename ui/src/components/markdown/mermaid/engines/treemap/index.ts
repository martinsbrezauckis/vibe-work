import type { DiagramEngine } from "../../types";
import { drawTreemap } from "./draw";
import { parseTreemap } from "./parse";

/** The Treemap diagram engine registered in groups/hierarchy.ts. */
export const engine: DiagramEngine = { render: (prepared) => drawTreemap(parseTreemap(prepared)) };
