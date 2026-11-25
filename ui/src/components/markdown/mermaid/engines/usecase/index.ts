/** Use case diagram engine entry. */
import type { DiagramEngine } from "../../types";
import { drawUsecase } from "./draw";
import { parseUsecase } from "./parse";

/** Use case diagram engine. */
export const engine: DiagramEngine = {
  render: (prepared) => drawUsecase(parseUsecase(prepared)),
};
