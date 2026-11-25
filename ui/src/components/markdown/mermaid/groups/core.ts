/** Registry slot "core": the families shipped with the framework. */
import type { GroupEngines } from "../types";

/** Engine loaders of the core group. */
export const engines: GroupEngines = {
  flowchart: () => import("../engines/flowchart").then((m) => m.engine),
  sequence: () => import("../engines/sequence").then((m) => m.engine),
  class: () => import("../engines/class").then((m) => m.engine),
  state: () => import("../engines/state").then((m) => m.engine),
  er: () => import("../engines/er").then((m) => m.engine),
  pie: () => import("../engines/pie").then((m) => m.engine),
  gantt: () => import("../engines/gantt").then((m) => m.engine),
};
