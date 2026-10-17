/**
 * The new-chat view's copy section: the "New copy" switch, and while it
 * is on the base branch and the branch name, suggested from the composer's text as the user types until the user
 * edits it by hand (then it stays). The first message is the composer's own text (the composer supplies the text).
 * Depends on: useCopyForm (state), IncludeList. Called by: sessions/NewChatPanel.
 * Invariants: renders only for a git folder, or with the reason alone when the core did not
 * read the folder because it was not chosen with Choose folder; a repository that is not offered a copy shows
 * the reason and a switch that cannot be turned on; names and messages are plain text inputs.
 */
import type { ReactElement } from "react";

import { Selector } from "../../components/Selector";
import { IncludeList } from "./IncludeList";
import type { CopyForm } from "./useCopyForm";

/** The section. */
export function CopySection({ form }: { form: CopyForm }): ReactElement | null {
  if (form.notPicked && form.existingBranch === null && form.reason !== null) {
    return (
      <fieldset className="copy-section" data-worktree="section">
        <p className="copy-reason" role="note">
          {form.reason}
        </p>
      </fieldset>
    );
  }
  if (!form.available) return null;
  return (
    <fieldset className="copy-section" data-worktree="section">
      <label className="copy-toggle">
        <input
          data-testid="worktrees.copy-section-set-on"
          type="checkbox"
          checked={form.on && form.offered}
          disabled={!form.offered}
          onChange={(e) => {
            form.setOn(e.target.checked);
          }}
        />
        New copy
      </label>
      {!form.offered && form.reason !== null && (
        <p className="copy-reason" role="note">
          {form.existingBranch !== null && form.notPicked ? "Choose folder" : form.reason}
        </p>
      )}
      {form.on && form.offered && (
        <div className="copy-fields">
          <label>
            Start from
            <Selector
              testId="worktrees.copy-section-set-base"
              label="Start from"
              value={form.base}
              options={[
                ...(form.detached === null
                  ? []
                  : [
                      {
                        value: form.detached,
                        label: `Current commit (${form.detached.slice(0, 8)}, detached)`,
                      },
                    ]),
                ...form.branches.map((branch) => ({
                  value: branch.name,
                  label: branch.current ? `${branch.name} (current)` : branch.name,
                })),
              ]}
              onChange={(value) => {
                form.setBase(value);
              }}
            />
          </label>
          <label>
            New branch
            <input
              data-testid="worktrees.copy-section-set-branch"
              type="text"
              className="mono"
              spellCheck={false}
              value={form.branch}
              onChange={(e) => {
                form.setBranch(e.target.value);
              }}
            />
          </label>
          {form.include?.found === true ? (
            <IncludeList report={form.include} plan />
          ) : (
            <p className="copy-reason" data-worktree="include-help">
              A new copy has none of your git-ignored files. To bring some along, list them in a
              file named .worktreeinclude at the repository root, one per line, for example .env or
              .env.local.
            </p>
          )}
        </div>
      )}
    </fieldset>
  );
}
