/** Event Modeling diagram engine entry. */
import type { DiagramEngine } from "../../types";
import { drawEventModeling } from "./draw";
import { parseEventModeling } from "./parse";

/** Event Modeling diagram engine. */
export const engine: DiagramEngine = {
  render: (prepared) => drawEventModeling(parseEventModeling(prepared)),
};
