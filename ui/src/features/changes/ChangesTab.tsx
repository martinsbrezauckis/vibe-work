/**
 * The Changes tab body: the shown session's changes in the view its folder allows: git review
 * per hunk, turn undo without git, or an honest empty or unavailable state. Loads on show; live updates
 * come from the store (`changesUpdated` events).
 * the branch bar (Commit, Push, Create PR, Bring into main, the PR card) sits on top, and the git
 * dialogs open from here.
 * Depends on: store.ts, GitReview, TurnUndo, ConfirmBox, shell hooks, git/BranchBar, git/GitDialogs. Called by:
 * register.tsx (the panel body).
 */
import { type ReactElement, useEffect } from "react";
import { EmptyState } from "../../components/EmptyState";

import { useStore } from "../../state/createStore";
import type { Git } from "../../state/git";
import { BranchBar } from "../git/BranchBar";
import { GitDialogs } from "../git/GitDialogs";
import { useApp } from "../shell/AppContext";
import { useSessionSummaries, useShellState } from "../shell/hooks";
import { ConfirmBox } from "./ConfirmBox";
import { GitReview } from "./GitReview";
import { LineCommentsPanel } from "./LineCommentsPanel";
import { LostBanner } from "./LostBanner";
import { historyOfferShown } from "./model";
import type { Changes } from "./store";
import { TurnUndo } from "./TurnUndo";

function Effects({
  git,
  changes,
  view,
  busy,
}: {
  git: Git | null;
  changes: Changes;
  view: import("../../protocol/core").ChangesView;
  busy: boolean;
}): ReactElement {
  return git === null ? (
    <TurnUndo changes={changes} view={view} busy={busy} foundRepo={false} />
  ) : (
    <EffectsWithGit git={git} changes={changes} view={view} busy={busy} />
  );
}

function EffectsWithGit({
  git,
  changes,
  view,
  busy,
}: {
  git: Git;
  changes: Changes;
  view: import("../../protocol/core").ChangesView;
  busy: boolean;
}): ReactElement {
  const foundRepo = useStore(
    git.store,
    (s) => !historyOfferShown(view, s.status[view.sessionId], s.selected[view.sessionId]),
  );
  return <TurnUndo changes={changes} view={view} busy={busy} foundRepo={foundRepo} />;
}

/** The tab. */
export function ChangesTab({ changes }: { changes: Changes }): ReactElement {
  const { git } = useApp();
  const sessionId = useShellState((s) => s.selected);
  const view = useStore(changes.store, (s) =>
    sessionId === null ? undefined : s.views[sessionId],
  );
  const busy = useStore(changes.store, (s) =>
    sessionId === null ? false : s.busy[sessionId] === true,
  );
  const error = useStore(changes.store, (s) =>
    sessionId === null ? null : (s.error[sessionId] ?? null),
  );
  const badge = useSessionSummaries().find((s) => s.id === sessionId)?.badge;
  const running = badge === "run" || badge === "input";
  useEffect(() => {
    if (sessionId !== null) void changes.load(sessionId);
  }, [changes, sessionId]);

  let body: ReactElement;
  if (sessionId === null) body = <div className="empty">Choose a session to see its changes.</div>;
  else if (view === undefined)
    // a first read that failed shows its error above and a way to read again, not a
    // "Reading changes…" that never ends.
    body =
      error === null ? (
        <div className="empty">Reading changes…</div>
      ) : (
        <div className="empty" data-testid="changes-read-failed">
          The changes could not be read.{" "}
          <button
            type="button"
            className="key-link"
            onClick={() => {
              void changes.load(sessionId);
            }}
          >
            Read again
          </button>
        </div>
      );
  else if (view.mode === "git")
    body = (
      <>
        {view.baselineLost && <LostBanner changes={changes} view={view} busy={busy} />}
        <GitReview changes={changes} view={view} busy={busy} running={running} />
      </>
    );
  else if (view.mode === "effects")
    body = <Effects git={git} changes={changes} view={view} busy={busy} />;
  else if (view.mode === "unavailable")
    body = <div className="empty">Changes cannot be shown: {view.reason ?? "unknown reason"}</div>;
  else
    body = (
      <EmptyState glyph="check" className="empty">
        No file changes in this session yet.
      </EmptyState>
    );
  return (
    <div
      className="changes-tab"
      data-mode={view?.mode ?? (error === null || sessionId === null ? "loading" : "failed")}
    >
      {error !== null && (
        <p className="changes-error" role="alert">
          {error}
        </p>
      )}
      {sessionId !== null && git !== null && <BranchBar git={git} sessionId={sessionId} />}
      {sessionId !== null && view?.mode === "git" && <LineCommentsPanel sessionId={sessionId} />}
      {body}
      <ConfirmBox changes={changes} />
      {sessionId !== null && git !== null && <GitDialogs git={git} sessionId={sessionId} />}
    </div>
  );
}
