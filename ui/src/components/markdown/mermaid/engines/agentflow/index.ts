/** Agentflow diagram engine entry. */
import type { DiagramEngine } from "../../types";
import { drawAgentflow } from "./draw";
import { parseAgentflow } from "./parse";

/** Agentflow diagram engine. */
export const engine: DiagramEngine = {
  render: (prepared) => drawAgentflow(parseAgentflow(prepared)),
};
