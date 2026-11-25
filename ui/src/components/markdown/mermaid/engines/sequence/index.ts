import type { DiagramEngine } from "../../types";
import { drawSequence } from "./draw";
import { parseSequence } from "./parse";

/** The engine registered in groups/core.ts: parse the prepared source, then draw it. */
export const engine: DiagramEngine = {
  render: (prepared) => drawSequence(parseSequence(prepared)),
};
