/**
 * The existing welcome list of Vibe Work copies without a live session: kept and leftover copies.
 * Depends on: worktrees state, AppContext and new-chat navigation. Called by: shell/Content.
 * Review uses the existing inspection and native removal dialog; New chat here reuses this folder.
 */
import { TEXT } from "../../vocabulary";
import { type ReactElement, useEffect } from "react";

import { useStore } from "../../state/createStore";
import type { Worktrees } from "../../state/worktrees";
import { useApp } from "../shell/AppContext";
import { openNewChat } from "../shell/newChat";

function List({ worktrees }: { worktrees: Worktrees }): ReactElement | null {
  const app = useApp();
  const copies = useStore(worktrees.store, (s) => s.orphans);
  useEffect(() => {
    void worktrees.loadOrphans();
  }, [worktrees]);
  // Hidden while there are none, like the sidebar's Leftover copies button (an empty "Copies 0" is noise).
  if (copies.length === 0) return null;
  return (
    <section
      data-testid="worktrees.orphan-list-section-surface-1"
      className="wt-orphans set-group"
      aria-label="Leftover copies"
      data-worktree="orphans"
      tabIndex={-1}
    >
      <h3>Leftover copies {copies.length}</h3>
      <ul>
        {copies.map((copy) => (
          <li
            className="check"
            data-item-id={copy.sessionId}
            key={copy.sessionId}
            data-copy-session={copy.sessionId}
          >
            <span>
              <span className="mono wt-path" title={copy.path ?? undefined}>
                {copy.path?.split(/[\\/]/).filter(Boolean).at(-1) ?? copy.branch}
              </span>
              <span className="d mono">{copy.branch}</span>
            </span>
            <span className="cactions">
              <span className="wt-status">{copy.kept ? "Kept" : "Leftover"}</span>
              <button
                data-testid="worktrees.orphan-list-review"
                type="button"
                className="tb-btn"
                onClick={() => void worktrees.review(copy)}
              >
                Review…
              </button>
              {copy.path !== null && (
                <button
                  data-testid="worktrees.orphan-list-open-new-chat"
                  type="button"
                  className="tb-btn"
                  onClick={() => {
                    openNewChat(app);
                    if (copy.path !== null) worktrees.newChatHere(copy.path);
                    document.querySelector(".new-session")?.scrollIntoView({ block: "center" });
                  }}
                >
                  {TEXT.newChat} here
                </button>
              )}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** The copies inventory in the welcome view. */
export function OrphanList(): ReactElement | null {
  const { worktrees } = useApp();
  return worktrees === null ? null : <List worktrees={worktrees} />;
}
