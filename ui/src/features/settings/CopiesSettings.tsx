/**
 * The General Settings entry to session copies (git worktrees): Review copies opens the welcome inventory of copies
 * no chat uses anymore; with none to review the row says so, and how many copies chats still use (the inventory
 * itself is hidden while empty).
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
    void worktrees.countCopies();
  }, [worktrees]);
  return count;
}

/** The row's text with nothing to review: copies chats still use are named, so it never reads as "no copies". */
function noneText(copies: number | null): string {
  if (copies === null || copies <= 0) return "None right now";
  return copies === 1
    ? "None to review: 1 in use by a chat"
    : `None to review: ${String(copies)} in use by chats`;
}

function Row({ worktrees }: { worktrees: Worktrees }) {
  const app = useApp();
  const count = useCopyCount(worktrees);
  const copies = useStore(worktrees.store, (s) => s.copies);
  return (
    <div className="check" data-setting="session-copies">
      <span>Chat copies</span>
      {count === 0 ? (
        <span className="d" data-testid="settings.copies-settings-none">
          {noneText(copies)}
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
