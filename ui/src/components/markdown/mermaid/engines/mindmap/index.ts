import type { DiagramEngine } from "../../types";
import { drawMindmap } from "./draw";
import { parseMindmap } from "./parse";

/** The Mindmap diagram engine registered in groups/hierarchy.ts. */
export const engine: DiagramEngine = { render: (prepared) => drawMindmap(parseMindmap(prepared)) };
