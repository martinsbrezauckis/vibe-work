import type { DiagramEngine } from "../../types";
import { drawVenn } from "./draw";
import { parseVenn } from "./parse";

/** The Venn diagram engine registered in groups/hierarchy.ts. */
export const engine: DiagramEngine = { render: (prepared) => drawVenn(parseVenn(prepared)) };
