/**
 * Registry slot "specialty": zenuml, swimlane, usecase, eventmodeling, agentflow, wardley, cynefin.
 * Add each family as a lazy loader. No other shared file changes.
 */
import type { GroupEngines } from "../types";

/** Lazy engine loaders of the specialty group. */
export const engines: GroupEngines = {
  cynefin: () => import("../engines/cynefin").then((m) => m.engine),
  eventmodeling: () => import("../engines/eventmodeling").then((m) => m.engine),
  wardley: () => import("../engines/wardley").then((m) => m.engine),
  zenuml: () => import("../engines/zenuml").then((m) => m.engine),
  swimlane: () => import("../engines/swimlane").then((m) => m.engine),
  usecase: () => import("../engines/usecase").then((m) => m.engine),
  agentflow: () => import("../engines/agentflow").then((m) => m.engine),
};
