import type { DiagramEngine } from "../../types";
import { drawGantt } from "./draw";
import { parseGantt } from "./parse";

/** The engine registered in groups/core.ts: parse the prepared source, then draw it. */
export const engine: DiagramEngine = { render: (prepared) => drawGantt(parseGantt(prepared)) };
