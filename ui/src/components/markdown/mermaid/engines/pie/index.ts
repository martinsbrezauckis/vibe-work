import type { DiagramEngine } from "../../types";
import { drawPie } from "./draw";
import { parsePie } from "./parse";

/** The pie chart engine registered in groups/core.ts. */
export const engine: DiagramEngine = { render: (prepared) => drawPie(parsePie(prepared)) };
