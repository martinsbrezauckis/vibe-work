/** Cynefin framework engine entry. */
import type { DiagramEngine } from "../../types";
import { drawCynefin } from "./draw";
import { parseCynefin } from "./parse";

/** Cynefin framework diagram engine. */
export const engine: DiagramEngine = {
  render: (prepared) => drawCynefin(parseCynefin(prepared)),
};
