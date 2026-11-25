/** Wardley Map engine entry. */
import type { DiagramEngine } from "../../types";
import { drawWardley } from "./draw";
import { parseWardley } from "./parse";

/** Wardley Map diagram engine. */
export const engine: DiagramEngine = {
  render: (prepared) => drawWardley(parseWardley(prepared)),
};
