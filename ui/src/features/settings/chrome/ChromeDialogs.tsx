/**
 * The Set up and Remove dialogs of Settings › Browser › Your Chrome.
 * - Set up, step 1: what Set up installs (the core's single-use offer: name, publisher, from, what it runs) and one
 *   Set up button that confirms exactly that offer; Cancel first. Step 2: "Add the Vibe Work extension in Chrome" with the
 *   live status line (re-read every few seconds while open), Open the store page again, and Turn on its tools (the
 *   existing Turn on with its own confirmation: giving the agent new tools keeps its question). While the store
 *   listing is in Google's review (`setup.storeReview`, from the signed release manifest), one line stands in place
 *   of the store button and the store instruction is hidden (it points at a page most people cannot open); both
 *   follow the core's `chromeControl` events, so a change shows without a restart.
 * - Remove: the destructive confirmation (Cancel first); then the status says to remove the extension in Chrome.
 * Depends on: Controls, useModalFocus, useChromeControl, chromeRows, the typed core client.
 * Called by: YourChromeSettings.
 */
import { TEXT } from "../../../vocabulary";
import { type ReactElement, useEffect, useRef, useState } from "react";

import { Button } from "../../../components/Controls";
import { useModalFocus } from "../../../hooks/useModalFocus";
import type { ToolsInstallOffer } from "../../../protocol/core";
import { useApp } from "../../shell/AppContext";
import { setupLine } from "./chromeRows";
import type { ChromeControlState } from "./useChromeControl";

/** How often step 2 reads the status again while it waits for the extension. */
const POLL_MS = 3000;

/** The Set up dialog. */
export function ChromeSetupDialog({
  open,
  chrome,
  onClose,
}: {
  open: boolean;
  chrome: ChromeControlState;
  onClose: () => void;
}): ReactElement | null {
  const app = useApp();
  const ref = useRef<HTMLDivElement>(null);
  const [offer, setOffer] = useState<ToolsInstallOffer | null>(null);
  const view = chrome.view;
  const setUp = view !== null && view.setup.state !== "notSetUp";
  const { run } = chrome;
  useModalFocus(open, ref, onClose);

  useEffect(() => {
    if (!open || setUp) return;
    let live = true;
    void run({ op: "offerSetup" }, "offerSetup").then((result) => {
      if (live && result?.type === "chromeControl") setOffer(result.offer);
    });
    return () => {
      live = false;
    };
  }, [open, setUp, run]);

  useEffect(() => {
    if (!open || !setUp || view.setup.pairing === "Paired") return;
    const timer = setInterval(() => {
      void app.core?.run({ type: "chromeControl", request: { op: "read" } }).catch(() => undefined);
    }, POLL_MS);
    return () => {
      clearInterval(timer);
    };
  }, [open, setUp, view, app.core]);

  if (!open) return null;
  const busy = chrome.busy !== null;
  return (
    <>
      <div className="scrim" data-testid="settings.chrome-setup-scrim" onClick={onClose} />
      <div
        className="modal"
        data-testid="settings.chrome-setup-dialog"
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby="chrome-setup-title"
        tabIndex={-1}
      >
        {!setUp ? (
          <>
            <header>
              <h2 id="chrome-setup-title">{TEXT.chromeSetupQuestion}</h2>
            </header>
            <section>
              {offer === null ? (
                <p className="key-line" role="status">
                  {chrome.status?.alert === true ? "" : "Reading what Set up installs…"}
                </p>
              ) : (
                <>
                  <p data-chrome-offer="">
                    <b>{offer.name}</b> · {offer.publisher ?? "Publisher not stated"} ·{" "}
                    {offer.source}
                  </p>
                  <p>{offer.runs}</p>
                </>
              )}
              {chrome.status?.alert === true ? (
                <p className="key-problem" role="alert">
                  {chrome.status.text}
                </p>
              ) : null}
            </section>
            <footer className="modal-actions">
              <Button
                data-testid="settings.chrome-setup-cancel"
                data-dialog-initial-focus="true"
                onClick={onClose}
              >
                Cancel
              </Button>
              <Button
                data-testid="settings.chrome-setup-confirm"
                variant="primary"
                disabled={offer === null || busy}
                onClick={() => {
                  if (offer !== null) void run({ op: "setup", offerId: offer.offerId }, "setup");
                }}
              >
                {chrome.busy === "setup" ? "Setting up…" : "Set up"}
              </Button>
            </footer>
          </>
        ) : (
          <>
            <header>
              <h2 id="chrome-setup-title">{TEXT.chromeAddExtension}</h2>
            </header>
            <section>
              {view.setup.storeReview ? null : (
                <p>
                  In the Chrome Web Store page, click Add to Chrome. Then open the extension and
                  pair with Vibe Work once. Compare the identity below with the one Chrome shows.
                </p>
              )}
              <p className="key-line" role="status" aria-live="polite" data-chrome-setup-status="">
                {setupLine(view)}
              </p>
              {view.setup.storeReview ? (
                <p className="key-line" role="status" data-chrome-store-review="">
                  Waiting for Google’s review of the extension.
                </p>
              ) : null}
              {view.setup.fingerprint !== null ? (
                <p className="key-line" data-chrome-fingerprint="">
                  <code>{view.setup.fingerprint}</code>
                </p>
              ) : null}
              {chrome.status?.alert === true ? (
                <p className="key-problem" role="alert">
                  {chrome.status.text}
                </p>
              ) : null}
            </section>
            <footer className="modal-actions">
              {view.setup.storeReview ? null : (
                <Button
                  data-testid="settings.chrome-setup-store"
                  data-dialog-initial-focus="true"
                  disabled={busy}
                  onClick={() => void run({ op: "openStore" }, "openStore")}
                >
                  Open the store page again
                </Button>
              )}
              <Button
                data-testid="settings.chrome-setup-close"
                data-dialog-initial-focus={view.setup.storeReview ? "true" : undefined}
                onClick={onClose}
              >
                Close
              </Button>
            </footer>
          </>
        )}
      </div>
    </>
  );
}

/** The Remove dialog. */
export function ChromeRemoveDialog({
  open,
  chrome,
  onClose,
}: {
  open: boolean;
  chrome: ChromeControlState;
  onClose: () => void;
}): ReactElement | null {
  const ref = useRef<HTMLDivElement>(null);
  useModalFocus(open, ref, onClose);
  if (!open) return null;
  return (
    <>
      <div className="scrim" data-testid="settings.chrome-remove-scrim" onClick={onClose} />
      <div
        className="modal"
        data-testid="settings.chrome-remove-dialog"
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby="chrome-remove-title"
        tabIndex={-1}
      >
        <header>
          <h2 id="chrome-remove-title">{TEXT.chromeRemoveQuestion}</h2>
        </header>
        <section>
          <p>{TEXT.chromeRemoveEffect}</p>
        </section>
        <footer className="modal-actions">
          <Button
            data-testid="settings.chrome-remove-cancel"
            data-dialog-initial-focus="true"
            onClick={onClose}
          >
            Cancel
          </Button>
          <Button
            data-testid="settings.chrome-remove-confirm"
            variant="danger"
            disabled={chrome.busy !== null}
            onClick={() => {
              void chrome.run({ op: "remove" }, "remove").then((result) => {
                if (result !== null) onClose();
              });
            }}
          >
            Remove
          </Button>
        </footer>
      </div>
    </>
  );
}
