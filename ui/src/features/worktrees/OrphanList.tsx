/**
 * The existing welcome list of Vibe Work copies without a live session: kept and leftover copies.
 * Depends on: worktrees state, AppContext and new-chat navigation. Called by: shell/Content.
 * Review uses the existing inspection and native removal dialog; New chat here reuses this folder.
 */
import { type ReactElement, useEffect } from "react";

import { useStore } from "../../state/createStore";
import type { Worktrees } from "../../state/worktrees";
import { useApp } from "../shell/AppContext";
import { openNewChat } from "../shell/newChat";

function List({ worktrees }: { worktrees: Worktrees }): ReactElement {
  const app = useApp();
  const copies = useStore(worktrees.store, (s) => s.orphans);
  useEffect(() => {
    void worktrees.loadOrphans();
  }, [worktrees]);
  return (
    <section className="wt-orphans" aria-label="Copies" data-worktree="orphans" tabIndex={-1}>
      <h3>Copies {copies.length}</h3>
      {copies.length === 0 && <span className="wt-status">No copies</span>}
      <ul>
        {copies.map((copy) => (
          <li key={copy.sessionId} data-copy-session={copy.sessionId}>
            <span className="mono">{copy.branch}</span>
            {copy.path !== null && (
              <span className="mono wt-path" title={copy.path}>
                {copy.path.split(/[\\/]/).filter(Boolean).at(-1) ?? copy.path}
              </span>
            )}
            <span className="wt-status">{copy.kept ? "Kept" : "Leftover"}</span>
            <button type="button" className="tb-btn" onClick={() => void worktrees.review(copy)}>
              Review…
            </button>
            {copy.path !== null && (
              <button
                type="button"
                className="tb-btn"
                onClick={() => {
                  openNewChat(app);
                  if (copy.path !== null) worktrees.newChatHere(copy.path);
                  document.querySelector(".new-session")?.scrollIntoView({ block: "center" });
                }}
              >
                New chat here
              </button>
            )}
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
