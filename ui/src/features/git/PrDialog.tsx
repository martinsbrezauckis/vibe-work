/**
 * The pull request dialog: title and body (from the branch's commits and the
 * repository's PR template), the base branch, Draft, and how it is made: with gh when installed and signed in,
 * else on the forge's own page in the system browser (GitHub, GitLab.com, Codeberg). It names the push that
 * happens first. The core then shows one native confirmation with every value, and runs exactly that.
 * Depends on: state/git, GitModal. Called by: GitDialogs.
 * Invariants: nothing leaves the machine from here; the press goes to the core's `createPr`.
 */
import { Button } from "../../components/Controls";
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
          <Button
            data-testid="git.pr-dialog-close"
            type="button"

            onClick={() => {
              git.close();
            }}
          >
            Cancel
          </Button>
          <Button
            data-testid="git.pr-dialog-create-pr"
            type="button"
            appearance="dialog"
            disabled={!can}
            data-action="pr-create"
            onClick={() => void git.createPr()}
          >
            {action}
          </Button>
        </>
      }
    >
      {dialog.repo !== "." && (
        <p className="ui-help" data-detail="paragraph">
          Repository: {dialog.repo}
        </p>
      )}
      {plan.existing !== null && (
        <p className="server-note" role="status">
          Pull request #{plan.existing.number} already exists for this branch; a new push updates
          it.
        </p>
      )}
      <p className="ui-help" data-detail="paragraph">
        {plan.via === "gh"
          ? `Made with gh on ${where}.`
          : plan.via === "browser"
            ? `Opens ${where} in your browser, filled in; you create it there.`
            : "This remote is not on GitHub or another site Vibe Work knows, so Vibe Work can push your branch but cannot open the pull request page. Open it on your server's site after pushing."}
        {plan.ghNote !== null && ` ${plan.ghNote}`}
      </p>
      {plan.push !== null && (
        <p className="ui-help" data-detail="paragraph">
          First it pushes {plan.push.branch} ({plan.push.commits}{" "}
          {plan.push.commits === 1 ? "commit" : "commits"}) to {plan.push.remote} ({plan.push.url}).
        </p>
      )}
      <label className="git-field">
        <span>Title</span>
        <input
          data-testid="git.pr-dialog-edit-pr"
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
            <span className="ui-help" data-detail="source">
              {" "}
              (template from {plan.template})
            </span>
          )}
        </span>
        <textarea
          data-testid="git.pr-dialog-edit-pr-2"
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
          data-testid="git.pr-dialog-edit-pr-3"
          value={dialog.base}
          data-git-field="base"
          onChange={(e) => {
            git.editPr({ base: e.target.value });
          }}
        />
      </label>
      <label className="git-check">
        <input
          data-testid="git.pr-dialog-edit-pr-4"
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
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
    </GitModal>
  );
}
