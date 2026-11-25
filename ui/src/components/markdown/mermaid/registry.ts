/** Maps a detected family to its engine through one lazy group module per registry slot. */
import { GROUP_OF } from "./detect";
import {
  DiagramUnsupportedError,
  type DiagramEngine,
  type DiagramGroup,
  type DiagramType,
  type GroupEngines,
} from "./types";

// Static dynamic imports so the bundler splits one chunk per group. Each group file exports
// `engines`, is the only file that changes when a family of that slot is added.
const groups: Record<DiagramGroup, () => Promise<{ engines: GroupEngines }>> = {
  core: () => import("./groups/core"),
  charts: () => import("./groups/charts"),
  timelines: () => import("./groups/timelines"),
  architecture: () => import("./groups/architecture"),
  hierarchy: () => import("./groups/hierarchy"),
  specialty: () => import("./groups/specialty"),
};

/** The engine for a family, or a DiagramUnsupportedError when no engine draws it yet. */
export async function engineFor(kind: DiagramType): Promise<DiagramEngine> {
  const { engines } = await groups[GROUP_OF[kind]]();
  const load = engines[kind];
  if (!load) throw new DiagramUnsupportedError(`Unsupported diagram: no drawing for ${kind} yet`);
  return load();
}
