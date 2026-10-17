/**
 * The Vibe part of the update area: a card next to the Vibe Work update notice ("Vibe Y is available"
 * with Update, "Vibe Y is out; support arrives in an upcoming update", a version notice for a user's own install,
 * the update's progress and "starts next time"), and the consent dialog Update opens: what will be downloaded
 * (sources with checksums, the commands, sizes) with "Update Vibe" and Cancel.
 * Depends on: AppContext, state/vibeUpdates (wording), state/vibeInstall (actions), hooks/useModalFocus,
 * setup/VibeOfferDetails. Called by: UpdateNotice (the card), Shell (the dialog).
 * Invariants: Update only asks the core for an offer; only the dialog's "Update Vibe" (the user's click) confirms
 * that exact offer. The dialog is modal (`aria-modal`: native views hide while it shows) and shows exactly while
 * the core holds an update offer.
 */
import { type ReactElement, useRef } from "react";

import { useModalFocus } from "../../hooks/useModalFocus";
import { type VibeNoticeAction, vibeNoticeFor } from "../../state/vibeUpdates";
import { useVibeUi } from "../setup/VibeInstallStep";
import { OfferDetails } from "../setup/VibeOfferDetails";
import { useApp } from "../shell/AppContext";

const LABELS: Record<VibeNoticeAction, string> = { update: "Update", later: "Later" };

/** The Vibe card of the update area, or nothing. */
export function VibeUpdateCard(): ReactElement | null {
  const { vibe } = useApp();
  const view = useVibeUi((s) => s.view);
  const dismissed = useVibeUi((s) => s.dismissed);
  const busy = useVibeUi((s) => s.busy);
  if (vibe === null || view === null) return null;
  const notice = vibeNoticeFor(view);
  if (notice === null || dismissed === notice.key) return null;
  return (
    <div
      className="update-notice"
      role="status"
      aria-live="polite"
      data-tone={notice.tone}
      data-testid="vibe-update-notice"
    >
      <div className="update-notice-text">
        <strong>{notice.headline}</strong>
        {notice.detail !== null && <span className="d">{notice.detail}</span>}
        {notice.progress !== null && (
          <progress max={1} value={notice.progress} aria-label="Vibe update progress" />
        )}
      </div>
      {notice.actions.length > 0 && (
        <div className="update-notice-actions">
          {notice.actions.map((action) => (
            <button
              data-testid="updates.vibe-update-prepare-update"
              data-item-id={action}
              key={action}
              type="button"
              className={action === "update" ? "tb-btn primary" : "tb-btn"}
              disabled={action === "update" && busy}
              onClick={() => {
                if (action === "update") void vibe.prepareUpdate();
                else vibe.dismiss(notice.key);
              }}
            >
              {LABELS[action]}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** The consent dialog, while the core holds a Vibe update offer. */
export function VibeUpdateDialog(): ReactElement | null {
  const { vibe } = useApp();
  const phase = useVibeUi((s) => s.view?.update.phase ?? null);
  const current = useVibeUi((s) =>
    s.view?.phase.state === "installed" ? s.view.phase.version : null,
  );
  const busy = useVibeUi((s) => s.busy);
  const dialog = useRef<HTMLDivElement>(null);
  const open = phase?.state === "offered";
  const cancel = () => {
    void vibe?.cancelUpdate();
  };
  useModalFocus(open, dialog, cancel);
  if (vibe === null || phase?.state !== "offered") return null;
  const offer = phase.offer;
  return (
    <>
      <div data-testid="updates.vibe-update-scrim" className="scrim" onClick={cancel} />
      <div
        className="modal vibe-update-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="vibe-update-title"
        data-testid="updates.vibe-update-dialog"
        ref={dialog}
        tabIndex={-1}
      >
        <header>
          <h2 id="vibe-update-title">Update Vibe to {offer.vibeVersion}</h2>
        </header>
        <section className="vibe-offer">
          <p>
            Vibe Work has tested Vibe {offer.vibeVersion}. It installs from Mistral&apos;s official
            package on PyPI into Vibe Work&apos;s own folder, next to your current Vibe
            {current === null ? "" : ` ${current}`}, which keeps running your chats. The new version
            starts the next time you open Vibe Work.
          </p>
          <OfferDetails offer={offer} />
        </section>
        <footer className="modal-actions">
          <button
            data-testid="updates.vibe-update-tb-btn"
            type="button"
            className="tb-btn"
            data-dialog-initial-focus="true"
            onClick={cancel}
          >
            Cancel
          </button>
          <button
            data-testid="updates.vibe-update-confirm-update"
            type="button"
            className="tb-btn primary"
            disabled={busy}
            onClick={() => void vibe.confirmUpdate(offer.offerId)}
          >
            Update Vibe
          </button>
        </footer>
      </div>
    </>
  );
}
