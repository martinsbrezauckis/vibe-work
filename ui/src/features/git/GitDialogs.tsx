/**
 * The open git dialog of the shown session, if any: Commit, Pull request or Bring into main.
 * Depends on: state/git, the three dialogs. Called by: changes/ChangesTab.
 */
import type { ReactElement } from "react";

import { useStore } from "../../state/createStore";
import type { Git } from "../../state/git";
import { BringDialog } from "./BringDialog";
import { CommitDialog } from "./CommitDialog";
import { PrDialog } from "./PrDialog";

/** The dialog for `sessionId`. */
export function GitDialogs({
  git,
  sessionId,
}: {
  git: Git;
  sessionId: string;
}): ReactElement | null {
  const dialog = useStore(git.store, (s) => s.dialog);
  const busy = useStore(git.store, (s) => s.busy[sessionId] === true);
  const notice = useStore(git.store, (s) => s.notice[sessionId] ?? null);
  if (dialog === null || dialog.sessionId !== sessionId) return null;
  const error = notice?.kind === "error" ? notice.text : null;
  switch (dialog.kind) {
    case "commit":
      return <CommitDialog git={git} dialog={dialog} busy={busy} error={error} />;
    case "pr":
      return <PrDialog git={git} dialog={dialog} busy={busy} error={error} />;
    case "bring":
      return <BringDialog git={git} dialog={dialog} busy={busy} error={error} />;
  }
}
