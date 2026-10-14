/**
 * Two sessions, one folder: when the folder chosen for a new session is a git
 * repository another loaded session is working in (running or with changes), the new-session view offers
 * "Work in its own copy (worktree)", so each session's Changes tab diffs against its own baseline and the
 * two never edit the same files.
 * Depends on: AppContext (core), protocol types. Called by: sessions/NewChatPanel.
 * Invariants: read only (`changes folderSessions`); a failed read shows no offer (starting in the same
 * folder stays possible).
 */
import { type ReactElement, useEffect, useState } from "react";

import type { FolderSession } from "../../protocol/core";
import { useApp } from "../shell/AppContext";

const NONE: readonly FolderSession[] = [];

/** Live sessions in `folder`'s repository (empty when none, when it is not git, or while unknown). */
export function useSharedFolder(folder: string | null): readonly FolderSession[] {
  const { core } = useApp();
  // Answers are kept with the folder they are for, so a changed folder never shows the old answer.
  const [found, setFound] = useState<{ folder: string; sessions: readonly FolderSession[] } | null>(
    null,
  );
  useEffect(() => {
    if (folder === null || core === null) return;
    let live = true;
    core
      .run({ type: "changes", request: { op: "folderSessions", cwd: folder } })
      .then((result) => {
        if (!live || result.type !== "changes" || result.result.kind !== "folder") return;
        setFound({ folder, sessions: result.result.isGit ? result.result.sessions : [] });
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [core, folder]);
  return found !== null && found.folder === folder ? found.sessions : NONE;
}

/** The note shown above the start buttons. */
export function SharedFolderNote({
  sessions,
}: {
  sessions: readonly FolderSession[];
}): ReactElement {
  const first = sessions[0];
  const name = first?.title ?? "another session";
  return (
    <p className="shared-note" role="note">
      {sessions.length === 1
        ? `"${name}" is working in this folder.`
        : `${String(sessions.length)} sessions are working in this folder.`}{" "}
      Give the new session its own copy (a git worktree), so neither changes the other&apos;s files.
    </p>
  );
}
