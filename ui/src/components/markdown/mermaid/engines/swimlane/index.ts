/** Swimlane diagram engine entry. */
import type { DiagramEngine } from "../../types";
import { drawSwimlane } from "./draw";
import { parseSwimlane } from "./parse";

/** Swimlane diagram engine. */
export const engine: DiagramEngine = {
  render: (prepared) => drawSwimlane(parseSwimlane(prepared)),
};
