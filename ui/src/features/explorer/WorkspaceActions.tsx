/**
 * Folder review and history initialization before a session exists. Reuses the Commit dialog and the existing
 * diff renderer; the steps and their state are the shared workspace model (`workspaceModel.ts`), which the
 * `ui/explorer` route drives too. Two instances render: the inline actions (`init` false: Review, Commit, Revert
 * and its confirmation) and the Turn on version history flow (`init` true: its plan dialog).
 */
import { useEffect } from "react";
import { Button } from "../../components/Controls";
import { GitModal } from "../git/GitModal";
import { GitDialogs } from "../git/GitDialogs";
import { GitIdentityFields } from "../git/GitIdentityFields";
import { validIdentity } from "../../state/gitIdentity";
import { GitReview } from "../changes/GitReview";
import type { App } from "../shell/AppContext";
import type { Explorer } from "./store";
import { useStore } from "../../state/createStore";
import { workspaceModelOf } from "./workspaceModel";

export function WorkspaceActions({
  app,
  explorer,
  init,
  onInitClose,
}: {
  app: App;
  explorer: Explorer;
  init: boolean;
  onInitClose: () => void;
}) {
  const model = workspaceModelOf(app, explorer);
  const status = useStore(explorer.store, (s) => s.status);
  const ws = useStore(model.store, (s) => s);
  const { busy, identity } = ws;
  const plan = init ? ws.plan : null;
  const review = init ? null : ws.review;
  const confirm = !init && ws.confirm;
  const error = ws.error;
  const key = model.key();
  useEffect(() => {
    if (init) model.openInit();
  }, [init, model]);
  const closeInit = () => {
    model.closeInit();
    onInitClose();
  };
  return (
    <>
      {init && !plan && (
        <p className="key-line" role="status">
          Reading files for the first commit…
        </p>
      )}
      {!init && status && (
        <div className="fx-workspace-actions">
          <Button
            data-testid="explorer.review-folder"
            disabled={busy}
            onClick={() => void model.review()}
          >
            Review all changes
          </Button>
          <Button
            data-testid="explorer.commit-folder"
            disabled={busy}
            onClick={() => void model.commit()}
          >
            Commit…
          </Button>
          <Button
            data-testid="explorer.ask-revert"
            variant="danger"
            disabled={busy}
            onClick={() => void model.askRevert()}
          >
            Revert all changes…
          </Button>
        </div>
      )}
      {review && !confirm && app.changes && (
        <section className="fx-workspace-review" aria-label="Changes since the last commit">
          <h3 className="ui-heading">Changes since the last commit</h3>
          <Button
            data-testid="explorer.close-review"
            onClick={() => {
              model.closeReview();
            }}
          >
            Close review
          </Button>
          <GitReview changes={app.changes} view={review.view} busy={busy} running={false} />
        </section>
      )}
      {confirm && review && (
        <GitModal
          title={`Revert all ${String(review.view.files.length)} changed files?`}
          kind="folder-revert"
          onClose={() => {
            model.cancelRevert();
          }}
          actions={
            <>
              <Button
                data-testid="explorer.cancel-revert"
                data-dialog-initial-focus="true"
                disabled={busy}
                onClick={() => {
                  model.cancelRevert();
                }}
              >
                Cancel
              </Button>
              <Button
                data-testid="explorer.confirm-revert"
                variant="danger"
                disabled={busy}
                onClick={() => void model.confirmRevert()}
              >
                {busy ? "Reverting…" : "Revert and delete"}
              </Button>
            </>
          }
        >
          <p>
            Tracked files return to the last commit. The new files listed below are deleted. Commits
            are kept.
          </p>
          <ul className="mono">
            {review.view.files.map((f) => (
              <li key={f.path}>
                {f.path}
                {f.status === "added" ? " · delete" : ""}
              </li>
            ))}
          </ul>
          {error && (
            <p className="key-problem" role="alert">
              {error}
            </p>
          )}
        </GitModal>
      )}
      {plan && (
        <GitModal
          title="Turn on version history?"
          kind="folder-init"
          onClose={closeInit}
          actions={
            <>
              <Button
                data-testid="explorer.cancel-history"
                data-dialog-initial-focus="true"
                disabled={busy}
                onClick={closeInit}
              >
                Cancel
              </Button>
              <Button
                data-testid="explorer.confirm-history"
                variant="primary"
                disabled={busy || !validIdentity(identity)}
                onClick={() =>
                  void model.confirmInit().then(() => {
                    if (!model.store.get().init) onInitClose();
                  })
                }
              >
                {busy ? "Creating history…" : "Turn on version history"}
              </Button>
            </>
          }
        >
          <p>Creates a Git repository and the first commit of {plan.files} files in this folder.</p>
          {plan.generated.length > 0 && (
            <p className="key-notice">
              Generated folders are included: {plan.generated.join(", ")}.
            </p>
          )}
          {plan.large.length > 0 && (
            <p className="key-notice">Large files are left out: {plan.large.join(", ")}.</p>
          )}
          <GitIdentityFields
            identity={identity}
            onChange={(next) => {
              model.setIdentity(next);
            }}
          />
          {error && (
            <p className="key-problem" role="alert">
              {error}
            </p>
          )}
        </GitModal>
      )}
      {error && !plan && !confirm && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
      {app.git && <GitDialogs git={app.git} sessionId={key} />}
    </>
  );
}
