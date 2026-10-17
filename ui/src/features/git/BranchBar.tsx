/**
 * The branch bar at the top of the Changes tab: `branch ← base · +N −M · files to
 * commit · [Commit] [Push] [Create PR]`, and for a session copy [Bring into <base>]. The buttons are always in
 * the bar, never in an overflow menu; a button that cannot act now is disabled with the reason as
 * its title. Below it: the last result or error, and the PR card when one is known.
 * Depends on: state/git (controller), model.ts, PrCard, Icon. Called by: changes/ChangesTab.
 * Invariants: the bar shows for every discovered repository, including clean and blocked repositories
 * (`barShown`); every button calls the controller, which asks the core (and the core asks the native dialog
 * before anything leaves the machine).
 */
import type { ReactElement } from "react";

import { Icon } from "../../components/Icon";
import { useStore } from "../../state/createStore";
import type { Git } from "../../state/git";
import { baseStat, barShown, branchOf, commitBlocked, prBlocked, pushBlocked } from "./model";
import { PrCard } from "./PrCard";

/** The bar for `sessionId`. */
export function BranchBar({
  git,
  sessionId,
}: {
  git: Git;
  sessionId: string;
}): ReactElement | null {
  const status = useStore(git.store, (s) => s.status[sessionId]);
  const busy = useStore(git.store, (s) => s.busy[sessionId] === true);
  const notice = useStore(git.store, (s) => s.notice[sessionId] ?? null);
  const loadError = useStore(git.store, (s) => s.loadError[sessionId] ?? null);
  const selected = useStore(git.store, (s) => s.selected[sessionId]);
  if (status === undefined || !barShown(status)) return null;
  const state = status.state;
  const branch = branchOf(status) ?? "detached";
  const base = state?.base ?? null;
  const stat = baseStat(status);
  const files = status.uncommitted ?? 0;
  const commitWhy = commitBlocked(status);
  const pushWhy = pushBlocked(status);
  const prWhy = prBlocked(status);
  const copy = status.location === "copy" && status.repo === ".";
  const bringWhy = copy && base?.ahead === 0 ? `${base.name} already has these changes.` : null;
  const reasons = [
    loadError === null ? null : "Git status unavailable.",
    commitWhy === null ? null : `Commit: ${commitWhy}`,
    pushWhy === null ? null : `Push: ${pushWhy}`,
    prWhy === null ? null : `Create PR: ${prWhy}`,
    bringWhy === null || base === null ? null : `Bring into ${base.name}: ${bringWhy}`,
  ].filter((reason): reason is string => reason !== null);
  return (
    <div className="git-bar" data-git="bar">
      <div className="git-line">
        {status.repos.length > 1 ? (
          <label className="git-repo">
            <span className="sr-only">Repository</span>
            <select
              data-testid="git.branch-bar-select-repo"
              aria-label="Repository"
              disabled={busy}
              value={selected ?? status.repo ?? ""}
              onChange={(event) => void git.selectRepo(sessionId, event.target.value)}
            >
              {status.repos.map((repo) => (
                <option
                  data-testid="git.branch-bar-option-surface-1"
                  data-item-id={repo.root}
                  key={repo.root}
                  value={repo.root}
                >
                  {repo.name}
                </option>
              ))}
            </select>
          </label>
        ) : status.repo !== "." && status.repos[0] !== undefined ? (
          <span className="git-repo-name" title={status.repos[0].root}>
            {status.repos[0].name}
          </span>
        ) : null}
        <span className="git-branch" title={state?.root ?? undefined}>
          <Icon name="git-branch" size={12} />
          <span className="mono">{branch}</span>
          {base !== null && (
            <span className="git-base">
              {" ← "}
              <span className="mono">{base.name}</span>
            </span>
          )}
        </span>
        {stat !== null && <span className="git-stat mono">{stat}</span>}
        {state?.upstream != null && (
          <span className="git-meta mono">
            → {state.upstream.remote}/{state.upstream.branch}
          </span>
        )}
        {base !== null && base.ahead > 0 && (
          <span className="git-meta">
            {base.ahead} {base.ahead === 1 ? "commit" : "commits"}
          </span>
        )}
        {files > 0 && (
          <span className="git-meta">
            {files} {files === 1 ? "file" : "files"} to commit
          </span>
        )}
        {status.toPush != null && status.toPush > 0 && (
          <span className="git-meta">
            {status.toPush} {status.toPush === 1 ? "commit" : "commits"} to push
          </span>
        )}
      </div>
      {reasons.length > 0 && <p className="git-reasons">{reasons.join(" · ")}</p>}
      <div className="git-actions" role="group" aria-label="Git">
        <button
          data-testid="git.branch-bar-open-commit"
          type="button"
          className="mini"
          data-action="git-commit"
          disabled={busy || loadError !== null || commitWhy !== null}
          title={loadError ?? commitWhy ?? "Commit the changes in this folder"}
          onClick={() => void git.openCommit(sessionId)}
        >
          Commit
        </button>
        <button
          data-testid="git.branch-bar-push"
          type="button"
          className="mini"
          data-action="git-push"
          disabled={busy || loadError !== null || pushWhy !== null}
          title={
            loadError ?? pushWhy ?? "Push this branch (you confirm the remote and branch first)"
          }
          onClick={() => void git.push(sessionId)}
        >
          Push
        </button>
        <button
          data-testid="git.branch-bar-open-pr"
          type="button"
          className="mini"
          data-action="git-pr"
          disabled={busy || loadError !== null || prWhy !== null}
          title={loadError ?? prWhy ?? "Create a pull request for this branch"}
          onClick={() => void git.openPr(sessionId)}
        >
          Create PR
        </button>
        {copy && base !== null && (
          <button
            data-testid="git.branch-bar-open-bring"
            type="button"
            className="mini"
            data-action="git-bring"
            disabled={busy || loadError !== null || base.ahead === 0}
            title={
              loadError ??
              (base.ahead === 0
                ? (bringWhy ?? undefined)
                : `Bring this copy's commits into ${base.name}`)
            }
            onClick={() => void git.openBring(sessionId)}
          >
            Bring into {base.name}
          </button>
        )}
      </div>
      {notice !== null && (
        <p
          className={notice.kind === "error" ? "key-problem" : "git-notice"}
          role={notice.kind === "error" ? "alert" : "status"}
        >
          {notice.text}
        </p>
      )}
      {loadError !== null && (
        <button
          data-testid="git.branch-bar-load"
          type="button"
          className="mini"
          onClick={() => void git.load(sessionId)}
        >
          Retry
        </button>
      )}
      {loadError === null && <PrCard git={git} sessionId={sessionId} />}
    </div>
  );
}
