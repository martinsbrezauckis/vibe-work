/**
 * The General Settings entry to session copies (git worktrees): Review copies opens the welcome inventory; with no
 * copies the row says so instead (the inventory itself is hidden while empty).
 * Depends on AppContext, shared controls, the worktrees store and openCopies; called by SettingsPage.
 */
import { useEffect } from "react";

import { Button } from "../../components/Controls";
import { useStore } from "../../state/createStore";
import type { Worktrees } from "../../state/worktrees";
import { useApp } from "../shell/AppContext";
import { openCopies } from "../worktrees/openCopies";

/** How many copies the inventory holds, read when Settings opens. */
function useCopyCount(worktrees: Worktrees): number {
  const count = useStore(worktrees.store, (s) => s.orphans.length);
  useEffect(() => {
    void worktrees.loadOrphans();
  }, [worktrees]);
  return count;
}

function Row({ worktrees }: { worktrees: Worktrees }) {
  const app = useApp();
  const count = useCopyCount(worktrees);
  return (
    <div className="check" data-setting="session-copies">
      <span>
        Session copies
        <span className="d">Review or remove session copies no chat uses anymore.</span>
      </span>
      {count === 0 ? (
        <span className="d" data-testid="settings.copies-settings-none">
          None right now
        </span>
      ) : (
        <Button
          data-testid="settings.copies-settings-open-copies"
          onClick={() => void openCopies(app)}
        >
          Review copies
        </Button>
      )}
    </div>
  );
}

/** Opens the same review list as the sessions footer. */
export function CopiesSettings() {
  const app = useApp();
  return (
    <section aria-labelledby="copies-settings-title">
      <h3 id="copies-settings-title">Git worktrees</h3>
      {app.worktrees === null ? null : <Row worktrees={app.worktrees} />}
    </section>
  );
}
