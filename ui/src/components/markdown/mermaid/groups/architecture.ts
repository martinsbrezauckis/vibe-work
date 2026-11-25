/**
 * Registry slot "architecture": c4, requirement, architecture, block, packet. Add each family as a lazy loader, e.g.
 * `quadrant: () => import("../engines/quadrant").then((m) => m.engine)`. No other shared file changes.
 */
import type { GroupEngines } from "../types";

/** Lazy engine loaders of the architecture group (c4, requirement, architecture, block, packet). */
export const engines: GroupEngines = {
  packet: () => import("../engines/packet").then((m) => m.engine),
  requirement: () => import("../engines/requirement").then((m) => m.engine),
  architecture: () => import("../engines/architecture").then((m) => m.engine),
  block: () => import("../engines/block").then((m) => m.engine),
  c4: () => import("../engines/c4").then((m) => m.engine),
};
