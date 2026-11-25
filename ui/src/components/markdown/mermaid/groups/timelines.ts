/**
 * Registry slot "timelines": journey, timeline, gitgraph, kanban. Add each family as a lazy loader, e.g.
 * `quadrant: () => import("../engines/quadrant").then((m) => m.engine)`. No other shared file changes.
 */
import type { GroupEngines } from "../types";

/** Lazy engine loaders of the timelines group (journey, timeline, gitgraph, kanban). */
export const engines: GroupEngines = {
  journey: () => import("../engines/journey").then((m) => m.engine),
  timeline: () => import("../engines/timeline").then((m) => m.engine),
  kanban: () => import("../engines/kanban").then((m) => m.engine),
  gitgraph: () => import("../engines/gitgraph").then((m) => m.engine),
};
