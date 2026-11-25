import type { DiagramEngine } from "../../types";
import { drawIshikawa } from "./draw";
import { parseIshikawa } from "./parse";

/** The Ishikawa (fishbone) diagram engine registered in groups/hierarchy.ts. */
export const engine: DiagramEngine = {
  render: (prepared) => drawIshikawa(parseIshikawa(prepared)),
};
