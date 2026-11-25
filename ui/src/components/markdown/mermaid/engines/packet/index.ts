import type { DiagramEngine } from "../../types";
import { drawPacket } from "./draw";
import { parsePacket } from "./parse";

/** The packet engine registered in groups/architecture.ts: parse prepared lines and draw SVG. */
export const engine: DiagramEngine = {
  render: (prepared) => drawPacket(parsePacket(prepared)),
};
