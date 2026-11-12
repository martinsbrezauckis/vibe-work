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

import type { NewFolderBy } from "../../state/shellState";
import { useStore } from "../../state/createStore";
import { NO_WORKTREES } from "../../state/worktrees";
import type { FolderSession } from "../../protocol/core";
import { useApp } from "../shell/AppContext";

const NONE: readonly FolderSession[] = [];

/** Live sessions in `folder`'s repository (empty when none, when it is not git, or while unknown). */
export function useSharedFolder(
  folder: string | null,
  by: NewFolderBy | null,
): { sessions: readonly FolderSession[]; pending: boolean } {
  const { core, worktrees } = useApp();
  const request = useStore(worktrees?.store ?? NO_WORKTREES, (s) => s.newChatRequest);
  // Answers are kept with the folder they are for, so a changed folder never shows the old answer.
  const [found, setFound] = useState<{
    folder: string;
    by: NewFolderBy | null;
    request: number;
    sessions: readonly FolderSession[];
  } | null>(null);
  useEffect(() => {
    if (folder === null || core === null) return;
    let live = true;
    core
      .run({ type: "changes", request: { op: "folderSessions", cwd: folder } })
      .then((result) => {
        if (!live) return;
        const sessions =
          result.type === "changes" && result.result.kind === "folder" && result.result.isGit
            ? result.result.sessions
            : NONE;
        setFound({ folder, by, request, sessions });
      })
      .catch(() => {
        if (live) setFound({ folder, by, request, sessions: NONE });
      });
    return () => {
      live = false;
    };
  }, [core, folder, by, request]);
  const current =
    found !== null && found.folder === folder && found.by === by && found.request === request;
  return {
    sessions: current ? found.sessions : NONE,
    pending: folder !== null && core !== null && !current,
  };
}

/** The note shown above the start buttons. */
export function SharedFolderNote({
  sessions,
}: {
  sessions: readonly FolderSession[];
}): ReactElement {
  const first = sessions[0];
  const name = first?.title ?? "another chat";
  return (
    <p className="shared-note" role="note">
      {sessions.length === 1
        ? `"${name}" is working in this folder.`
        : `${String(sessions.length)} chats are working in this folder.`}{" "}
      Give the new chat its own copy (a git worktree), so neither changes the other&apos;s files.
    </p>
  );
}
