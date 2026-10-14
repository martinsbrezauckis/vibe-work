/**
 * The pull request dialog: title and body (from the branch's commits and the
 * repository's PR template), the base branch, Draft, and how it is made: with gh when installed and signed in,
 * else on the forge's own page in the system browser (GitHub, GitLab.com, Codeberg). It names the push that
 * happens first. The core then shows one native confirmation with every value, and runs exactly that.
 * Depends on: state/git, GitModal. Called by: GitDialogs.
 * Invariants: nothing leaves the machine from here; the press goes to the core's `createPr`.
 */
import type { ReactElement } from "react";

import type { Git, PrDialog as Dialog } from "../../state/git";
import { GitModal } from "./GitModal";

/** The open PR dialog. */
export function PrDialog({
  git,
  dialog,
  busy,
  error,
}: {
  git: Git;
  dialog: Dialog;
  busy: boolean;
  error: string | null;
}): ReactElement {
  const { plan } = dialog;
  const where = plan.forge === null ? plan.remote : `${plan.forge.host}/${plan.forge.path}`;
  const action =
    plan.via === "gh"
      ? "Create pull request"
      : plan.via === "browser"
        ? "Continue on the forge's page"
        : "Push";
  const can = !busy && dialog.title.trim() !== "" && dialog.base.trim() !== "";
  return (
    <GitModal
      title={`Pull request from ${plan.head}`}
      kind="pr"
      onClose={() => {
        git.close();
      }}
      actions={
        <>
          <button
            type="button"
            className="tb-btn"
            onClick={() => {
              git.close();
            }}
          >
            Cancel
          </button>
          <button
            type="button"
            className="btn-primary"
            disabled={!can}
            data-action="pr-create"
            onClick={() => void git.createPr()}
          >
            {action}
          </button>
        </>
      }
    >
      {dialog.repo !== "." && <p className="git-note">Repository: {dialog.repo}</p>}
      {plan.existing !== null && (
        <p className="git-warn">
          Pull request #{plan.existing.number} already exists for this branch; a new push updates
          it.
        </p>
      )}
      <p className="git-note">
        {plan.via === "gh"
          ? `Made with gh on ${where}.`
          : plan.via === "browser"
            ? `Opens ${where} in your browser, filled in; you create it there.`
            : `Vibe Work has no page for ${where}: it pushes, then you open your forge.`}
        {plan.ghNote !== null && ` ${plan.ghNote}`}
      </p>
      {plan.push !== null && (
        <p className="git-note">
          First it pushes {plan.push.branch} ({plan.push.commits}{" "}
          {plan.push.commits === 1 ? "commit" : "commits"}) to {plan.push.remote} ({plan.push.url}).
        </p>
      )}
      <label className="git-field">
        <span>Title</span>
        <input
          value={dialog.title}
          data-dialog-initial-focus="true"
          data-git-field="title"
          onChange={(e) => {
            git.editPr({ title: e.target.value });
          }}
        />
      </label>
      <label className="git-field">
        <span>
          Description
          {plan.template !== null && (
            <span className="git-from"> (template from {plan.template})</span>
          )}
        </span>
        <textarea
          rows={8}
          value={dialog.body}
          data-git-field="body"
          onChange={(e) => {
            git.editPr({ body: e.target.value });
          }}
        />
      </label>
      <label className="git-field git-inline">
        <span>Into</span>
        <input
          value={dialog.base}
          data-git-field="base"
          onChange={(e) => {
            git.editPr({ base: e.target.value });
          }}
        />
      </label>
      <label className="git-check">
        <input
          type="checkbox"
          checked={dialog.draft}
          data-git-field="draft"
          onChange={(e) => {
            git.editPr({ draft: e.target.checked });
          }}
        />
        <span>
          Draft
          {plan.via === "browser" &&
            plan.forge?.kind === "gitHub" &&
            ' (on GitHub\'s page, choose "Create draft pull request")'}
        </span>
      </label>
      {error !== null && (
        <p className="git-warn" role="alert">
          {error}
        </p>
      )}
    </GitModal>
  );
}
