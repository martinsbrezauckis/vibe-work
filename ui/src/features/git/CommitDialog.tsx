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
import type { ReactElement } from "react";

import type { CommitDialog as Dialog, Git } from "../../state/git";
import { GitModal } from "./GitModal";
import { KeyNotice } from "../shell/KeyNotice";

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
  const { plan, message, excluded, runHooks } = dialog;
  const chosen = plan.files.filter((f) => !excluded.includes(f.path));
  const blocked = plan.blocker ?? plan.authorProblem;
  const can = !busy && blocked === null && chosen.length > 0 && message.trim() !== "";
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
      {blocked !== null && (
        <p className="git-warn" role="alert">
          {blocked}
        </p>
      )}
      {dialog.repo !== "." ? (
        <p className="git-note">Repository: {dialog.repo}</p>
      ) : plan.note !== null ? (
        <p className="git-note">{plan.note}</p>
      ) : null}
      <KeyNotice message="Drafting a commit message uses your Mistral key; you can still write and commit a message yourself." />
      <label className="git-field">
        <span>
          Message{" "}
          <span className="git-from">
            {dialog.messageFrom === "loading"
              ? "(writing a draft…)"
              : dialog.messageFrom === "model"
                ? "(drafted by Mistral; edit freely)"
                : dialog.messageFrom === "fallback"
                  ? `(from the session title${dialog.messageNote === null ? "" : `. ${dialog.messageNote}`})`
                  : ""}
          </span>
        </span>
        <textarea
          rows={5}
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
          <li key={f.path}>
            <label>
              <input
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
      ) : (
        <p className="git-sub">This repository has no commit hooks.</p>
      )}
      <details className="git-rule">
        <summary>
          Message rule for this project{plan.instruction.trim() !== "" ? " (set)" : ""}
        </summary>
        <textarea
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
        <p className="git-warn" role="alert">
          {error}
        </p>
      )}
    </GitModal>
  );
}
