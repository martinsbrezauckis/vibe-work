/** Session-local work admission. Uses live core and composer state; Git controls and commands share it without waiting for Git reads. */
import type { CoreState } from "./coreModel";
import type { ComposerState } from "./composerStore";
import { statusType } from "./sessionSummary";

/** A send in flight, a manual compaction, or this chat's running/blocked turn keeps its files busy. */
export function sessionWorking(
  core: CoreState,
  composer: ComposerState | undefined,
  id: string,
): boolean {
  return (
    composer?.sending[id] === true ||
    composer?.compacting[id] === true ||
    ["running", "blocked"].includes(statusType(core.sessions[id]?.session ?? {}))
  );
}
