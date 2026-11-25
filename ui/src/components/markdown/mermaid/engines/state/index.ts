import type { DiagramEngine } from "../../types";
import { drawState } from "./draw";
import { parseState } from "./parse";

/** The engine registered in groups/core.ts: parse the prepared source, then draw it. */
export const engine: DiagramEngine = {
  render: (prepared) => drawState(parseState(prepared)),
};
