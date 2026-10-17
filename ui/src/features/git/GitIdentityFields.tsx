/** Shared author fields and a warning-only email hint; rendered by Settings, Commit and repository init. */
import { useId } from "react";
import type { CommitIdentity } from "../../protocol/core";
import { unusualEmail } from "../../state/gitIdentity";

/** Plain author fields on the shared key-field style (Settings, dialogs), without browser form validation. */
export function GitIdentityFields({
  identity,
  onChange,
  disabled = false,
}: {
  identity: CommitIdentity;
  onChange: (identity: CommitIdentity) => void;
  disabled?: boolean;
}) {
  const id = useId();
  const hint = `${id}-email-hint`;
  return (
    <div className="git-identity">
      <div className="key-field">
        <label htmlFor={`${id}-name`}>Name</label>
        <div className="key-field-row">
          <input
            id={`${id}-name`}
            data-testid="git.identity-name"
            data-git-field="author-name"
            autoComplete="name"
            value={identity.name}
            disabled={disabled}
            onChange={(e) => {
              onChange({ ...identity, name: e.target.value });
            }}
          />
        </div>
      </div>
      <div className="key-field">
        <label htmlFor={`${id}-email`}>Email</label>
        <div className="key-field-row">
          <input
            id={`${id}-email`}
            data-testid="git.identity-email"
            data-git-field="author-email"
            type="text"
            inputMode="email"
            autoComplete="email"
            aria-describedby={unusualEmail(identity.email) ? hint : undefined}
            value={identity.email}
            disabled={disabled}
            onChange={(e) => {
              onChange({ ...identity, email: e.target.value });
            }}
          />
        </div>
      </div>
      {unusualEmail(identity.email) && (
        <p id={hint} className="server-note" role="status">
          This email looks unusual. You can still use it.
        </p>
      )}
    </div>
  );
}
