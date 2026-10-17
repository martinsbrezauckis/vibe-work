/**
 * The new-chat view's folder and copy controls (New chat; replaces the start panel): the
 * folder chip with "Choose folder…", the note about other live sessions in the repository, and the "Work in a new
 * copy (worktree)" section. There is no Start button: the composer under it (key `NEW_CHAT`) starts the session on
 * its first send (`state/composer.ts`), in this folder or, with the switch on, in a new copy whose branch name is
 * suggested from the composer's text until the user edits it.
 * The folder text is shown, never typed: only the native dialog (owned by Rust) can produce a folder the gate
 * treats as picked. Starting a session does not trust the folder; trust is its own action in the project menu.
 * Depends on: AppContext, hooks, components/Icon, changes/SharedFolder (another live session in the repository),
 * worktrees/useCopyForm + CopySection, state/composer (`setNewChatStarter`). Called by: shell/Content.
 * Invariants: while mounted it installs the starter Send uses (copy when the switch is on, else the folder itself);
 * a copy without a branch name refuses the send with a reason (the text stays). When another live session works in
 * the folder's repository the copy switch is on by default. A folder a Debug API client
 * put here through `ui/worktrees folder` is treated as picked and reports its origin, and starts for it are marked as the Debug API's
 * (a label; a later action runs as the window after the client request ends); a native
 * pick replaces it and ends the mark. After a Send without a folder the chip lights up (`askFolder`) until a folder
 * is chosen.
 */
import { type ReactElement, useEffect, useRef } from "react";

import { Icon } from "../../components/Icon";
import { PathText } from "../../components/PathText";
import { NEW_CHAT } from "../../state/newChatKey";
import { SharedFolderNote, useSharedFolder } from "../changes/SharedFolder";
import { useComposerState } from "../composer/hooks";
import { useApp } from "../shell/AppContext";
import { useShellState } from "../shell/hooks";
import { CopySection } from "../worktrees/CopySection";
import { useStore } from "../../state/createStore";
import { NO_SESSION_TOOLS } from "../../state/sessionTools";
import { TemporaryNote, TemporarySwitch } from "./TemporarySwitch";
import { type CopyForm, useCopyForm } from "../worktrees/useCopyForm";

/** The folder chip, the shared-folder note and the copy section of the new-chat view. */
export function NewChatPanel(): ReactElement | null {
  const { sessions, worktrees, composer, sessionTools } = useApp();
  const temporary = useStore(sessionTools?.store ?? NO_SESSION_TOOLS, (s) => s.newChatTemporary);
  const folder = useShellState((s) => s.newFolder);
  const by = useShellState((s) => s.newFolderBy);
  const draft = useComposerState((s) => s.drafts[NEW_CHAT] ?? "");
  const ask = useComposerState((s) => s.newChat.askFolder);
  const others = useSharedFolder(folder);
  const copy = useCopyForm(folder, by, others.length > 0, worktrees);
  // The branch name is suggested from what the user types into the composer (the copy's first message).
  const { setMessage } = copy;
  useEffect(() => {
    setMessage(draft);
  }, [draft, setMessage]);
  // A chosen folder answers the "choose a folder first" hint.
  useEffect(() => {
    if (folder !== null && composer.store.get().newChat.askFolder) {
      composer.holdNewChat({ askFolder: false });
      composer.note(NEW_CHAT, null);
    }
  }, [folder, composer]);
  // The starter reads the latest form through a ref (installed once per mount, never a stale form).
  const latest = useRef<{ copy: CopyForm; folder: string | null; by: typeof by }>({
    copy,
    folder,
    by,
  });
  useEffect(() => {
    latest.current = { copy, folder, by };
  });
  useEffect(() => {
    if (sessions === null) return;
    composer.setNewChatStarter({
      refuse() {
        const { copy: form } = latest.current;
        return form.active && form.branch.trim() === ""
          ? "Name the new branch for the copy first. Your text stays here."
          : null;
      },
      start(origin) {
        const { copy: form, folder: cwd, by: chosenBy } = latest.current;
        if (cwd === null) return Promise.resolve(null);
        const viaApi = origin === "debug-api" || chosenBy === "debug-api" ? "debug-api" : undefined;
        // a temporary chat starts in the folder itself (never in a copy).
        if (sessionTools?.store.get().newChatTemporary === true)
          return sessionTools.startTemporary(cwd, viaApi);
        return form.active ? form.start(viaApi) : sessions.startIn(cwd, viaApi);
      },
    });
    return () => {
      composer.setNewChatStarter(null);
    };
  }, [composer, sessions, sessionTools]);
  if (sessions === null) return null;
  return (
    <div className="new-session" data-ask-folder={ask ? "true" : undefined}>
      <div className="folder-row">
        <button
          data-testid="sessions.new-chat-panel-pick-folder"
          type="button"
          className="tb-btn"
          aria-describedby={ask ? "new-chat-folder-hint" : undefined}
          onClick={() => void sessions.pickFolder()}
        >
          <Icon name="folder" />
          {folder === null ? "Choose folder..." : "Change folder..."}
        </button>
        {folder !== null && (
          <PathText
            path={
              copy.existingBranch === null
                ? folder
                : `${folder.split(/[\\/]/).filter(Boolean).at(-1) ?? folder} · ${copy.existingBranch}`
            }
            title={folder}
            className="mono folder-path folder-chip"
            testId="new-chat-folder"
          />
        )}
        {sessionTools !== null && (
          <TemporarySwitch
            on={temporary}
            onChange={(on) => {
              sessionTools.setNewChatTemporary(on);
            }}
          />
        )}
        {ask && folder === null && (
          <span className="folder-hint" id="new-chat-folder-hint" role="status">
            Choose the folder this session works in.
          </span>
        )}
      </div>
      {folder !== null && by === "debug-api" && (
        <p className="copy-reason" role="note" data-folder-by="debug-api">
          An external client (the Debug API) chose this folder, not you.
        </p>
      )}
      {others.length > 0 && <SharedFolderNote sessions={others} />}
      {temporary && <TemporaryNote />}
      {folder !== null && !temporary && <CopySection form={copy} />}
    </div>
  );
}
