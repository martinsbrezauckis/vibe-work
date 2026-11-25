import type { DiagramEngine } from "../../types";
import { drawBlock } from "./draw";
import { parseBlock } from "./parse";

/** The block engine registered in groups/architecture.ts: parse prepared lines and draw SVG. */
export const engine: DiagramEngine = {
  render: (prepared) => drawBlock(parseBlock(prepared)),
};
