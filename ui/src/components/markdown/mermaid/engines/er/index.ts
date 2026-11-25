import type { DiagramEngine } from "../../types";
import { drawEr } from "./draw";
import { parseEr } from "./parse";

/** The engine registered in groups/core.ts: parse the prepared source, then draw it. */
export const engine: DiagramEngine = { render: (prepared) => drawEr(parseEr(prepared)) };
