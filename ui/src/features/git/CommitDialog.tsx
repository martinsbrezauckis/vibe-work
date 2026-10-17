/**
 * The Commit dialog: the message (drafted by Mistral in the repository's style
 * and the project's instruction, or the session title without a key; the user can type over it), the exact
 * files that go in (git's own list, each can be left out), the author, the repository's commit hooks with the
 * per-project switch, and Commit · Commit and push · Commit and create PR. Ctrl/Cmd+Enter = Commit.
 * Depends on: state/git (dialog state, actions), GitModal. Called by: GitDialogs.
 * Invariants: what the dialog lists is what is committed (the core refuses a list or HEAD that moved, `stale`);
 * nothing is sent without a button press; hook output appears after the commit (and as the error when a hook
 * stops it).
 */
import { useId, type ReactElement } from "react";

import type { CommitDialog as Dialog, Git } from "../../state/git";
import { GitModal } from "./GitModal";
import { KeyNotice } from "../shell/KeyNotice";
import { GitIdentityFields } from "./GitIdentityFields";
import { validIdentity } from "../../state/gitIdentity";

const STATUS_WORD: Record<string, string> = {
  modified: "changed",
  added: "new",
  deleted: "deleted",
  typeChanged: "type changed",
  submodule: "submodule",
  conflicted: "conflict",
};

/** The open Commit dialog. */
export function CommitDialog({
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
  const { plan, message, excluded, runHooks, authorName, authorEmail } = dialog;
  const messageNoteId = useId();
  const fallback = dialog.messageFrom === "fallback" && dialog.messageNote !== null;
  const chosen = plan.files.filter((f) => !excluded.includes(f.path));
  const blocked = plan.blocker;
  const needsAuthor = plan.authorProblem !== null;
  const validAuthor = validIdentity({ name: authorName, email: authorEmail });
  const can =
    !busy &&
    blocked === null &&
    (!needsAuthor || validAuthor) &&
    chosen.length > 0 &&
    message.trim() !== "";
  const toggle = (path: string) => {
    git.editCommit({
      excluded: excluded.includes(path) ? excluded.filter((p) => p !== path) : [...excluded, path],
    });
  };
  const commit = (then: "none" | "push" | "pr") => {
    if (can) void git.commit(then);
  };
  return (
    <GitModal
      title={`Commit to ${plan.branch}`}
      kind="commit"
      onClose={() => {
        git.close();
      }}
      actions={
        <>
          <button
            data-testid="git.commit-dialog-close"
            type="button"
            className="tb-btn"
            onClick={() => {
              git.close();
            }}
          >
            Cancel
          </button>
          <button
            data-testid="git.commit-dialog-commit"
            type="button"
            className="tb-btn"
            disabled={!can}
            data-action="commit-pr"
            onClick={() => {
              commit("pr");
            }}
          >
            Commit and create PR
          </button>
          <button
            data-testid="git.commit-dialog-commit-2"
            type="button"
            className="tb-btn"
            disabled={!can}
            data-action="commit-push"
            onClick={() => {
              commit("push");
            }}
          >
            Commit and push
          </button>
          <button
            data-testid="git.commit-dialog-commit-3"
            type="button"
            className="btn-primary"
            disabled={!can}
            data-action="commit"
            onClick={() => {
              commit("none");
            }}
          >
            Commit
          </button>
        </>
      }
    >
      <p className="git-note">
        Saves a version of the ticked files on {plan.branch}. The list shows every changed file in
        this folder, also changes you made yourself; untick what should not be saved.
      </p>
      {blocked !== null && (
        <p className="key-problem" role="alert">
          {blocked}
        </p>
      )}
      {dialog.repo !== "." && <p className="git-note">Repository: {dialog.repo}</p>}
      <KeyNotice message="Drafting a commit message uses your Mistral key; you can still write and commit a message yourself." />
      {needsAuthor && (
        <>
          <p className="git-note">
            Enter your name and email for this commit. They are saved only in this repository.
          </p>
          <GitIdentityFields
            identity={{ name: authorName, email: authorEmail }}
            onChange={(identity) => {
              git.editCommit({ authorName: identity.name, authorEmail: identity.email });
            }}
          />
        </>
      )}
      <label className="git-field">
        <span>Message</span>
        {fallback && (
          <span className="git-from" id={messageNoteId}>
            Written from the chat title. The automatic message is not available:{" "}
            {(dialog.messageNote ?? "").replace(/[.\s]+$/, "")}.
          </span>
        )}
        <textarea
          data-testid="git.commit-dialog-edit-commit"
          rows={5}
          aria-label="Message"
          aria-describedby={fallback ? messageNoteId : undefined}
          value={message}
          data-dialog-initial-focus="true"
          data-git-field="message"
          onChange={(e) => {
            git.editCommit({ message: e.target.value });
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
              e.preventDefault();
              commit("none");
            }
          }}
        />
      </label>
      <p className="git-sub">
        {chosen.length} of {plan.files.length} {plan.files.length === 1 ? "file" : "files"}
        {plan.author !== null && <> · author {plan.author}</>}
      </p>
      <ul className="git-files" aria-label="Files in this commit">
        {plan.files.map((f) => (
          <li data-item-id={f.path} key={f.path}>
            <label>
              <input
                data-testid="git.commit-dialog-toggle"
                type="checkbox"
                checked={!excluded.includes(f.path)}
                onChange={() => {
                  toggle(f.path);
                }}
              />
              <span className="mono">{f.path}</span>
              <span className="git-st">
                {STATUS_WORD[f.status] ?? f.status}
                {f.staged ? " · staged" : ""}
              </span>
            </label>
          </li>
        ))}
      </ul>
      {plan.hooks.hooks.length > 0 ? (
        <label className="git-check">
          <input
            data-testid="git.commit-dialog-edit-commit-2"
            type="checkbox"
            checked={runHooks}
            data-git-field="run-hooks"
            onChange={(e) => {
              git.editCommit({ runHooks: e.target.checked });
              void git.saveSetting(dialog.sessionId, { runHooks: e.target.checked });
            }}
          />
          <span>
            Run this repository&apos;s commit hooks: {plan.hooks.hooks.join(", ")} (from{" "}
            {plan.hooks.folder}). They run as you, as with git commit in a terminal.
          </span>
        </label>
      ) : null}
      <details className="git-rule">
        <summary data-testid="git.commit-dialog-summary">
          Message rule for this project{plan.instruction.trim() !== "" ? " (set)" : ""}
        </summary>
        <textarea
          data-testid="git.commit-dialog-textarea"
          rows={3}
          defaultValue={plan.instruction}
          data-git-field="instruction"
          aria-label="Message rule for this project"
          placeholder="For example: Conventional Commits, English, imperative mood, mention the issue number."
          onBlur={(e) => {
            if (e.target.value !== plan.instruction)
              void git
                .saveSetting(dialog.sessionId, { instruction: e.target.value })
                .then(async () => {
                  await git.reopenCommit(dialog.sessionId);
                  await git.draft();
                });
          }}
        />
      </details>
      <label className="git-check">
        <input
          data-testid="git.commit-dialog-save-setting"
          type="checkbox"
          checked={plan.credit}
          data-git-field="credit"
          onChange={(e) => {
            void git
              .saveSetting(dialog.sessionId, { credit: e.target.checked })
              .then(() => git.reopenCommit(dialog.sessionId));
          }}
        />
        <span>
          Add &quot;Co-Authored-By: Mistral Vibe&quot; to my commit messages in this project
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
