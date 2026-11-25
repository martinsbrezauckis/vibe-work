/**
 * Registry slot "hierarchy": mindmap, treemap, ishikawa, treeview (venn once its header is confirmed). Add each family as a lazy loader, e.g.
 * `quadrant: () => import("../engines/quadrant").then((m) => m.engine)`. No other shared file changes.
 */
import type { GroupEngines } from "../types";

/** Lazy engine loaders of the hierarchy group (mindmap, treemap, ishikawa, treeview, venn). */
export const engines: GroupEngines = {
  ishikawa: () => import("../engines/ishikawa").then((m) => m.engine),
  treeview: () => import("../engines/treeview").then((m) => m.engine),
  mindmap: () => import("../engines/mindmap").then((m) => m.engine),
  treemap: () => import("../engines/treemap").then((m) => m.engine),
  venn: () => import("../engines/venn").then((m) => m.engine),
};
