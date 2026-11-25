/**
 * Registry slot "charts": quadrant, xychart, sankey, radar. Add each family as a lazy loader, e.g.
 * `quadrant: () => import("../engines/quadrant").then((m) => m.engine)`. No other shared file changes.
 */
import type { GroupEngines } from "../types";

/** Lazy engine loaders of the charts group (quadrant, xychart, sankey, radar). */
export const engines: GroupEngines = {
  quadrant: () => import("../engines/quadrant").then((m) => m.engine),
  radar: () => import("../engines/radar").then((m) => m.engine),
  xychart: () => import("../engines/xychart").then((m) => m.engine),
  sankey: () => import("../engines/sankey").then((m) => m.engine),
};
