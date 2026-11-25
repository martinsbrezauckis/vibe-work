import type { DiagramEngine } from "../../types";
import { drawGitGraph } from "./draw";
import { parseGitGraph } from "./parse";

/** The engine registered in groups/timelines.ts: parse the prepared source, then draw it. */
export const engine: DiagramEngine = {
  render: (prepared) => drawGitGraph(parseGitGraph(prepared)),
};
