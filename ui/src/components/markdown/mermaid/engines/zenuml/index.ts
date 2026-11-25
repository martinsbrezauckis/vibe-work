/** ZenUML diagram engine entry. */
import type { DiagramEngine } from "../../types";
import { drawZenuml } from "./draw";
import { parseZenuml } from "./parse";

/** ZenUML sequence diagram engine. */
export const engine: DiagramEngine = {
  render: (prepared) => drawZenuml(parseZenuml(prepared)),
};
