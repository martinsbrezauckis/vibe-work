/**
 * The Set up and Remove dialogs of Settings › Browser › Your Chrome.
 * - Set up, step 1: what Set up installs (the core's single-use offer: name, publisher, from, what it runs) and one
 *   Set up button that confirms exactly that offer; Cancel first. Step 2: "Add the extension in Chrome" with the
 *   live status line (re-read every few seconds while open), Open the store page again, and Turn on its tools (the
 *   existing Turn on with its own confirmation: giving the agent new tools keeps its question).
 * - Remove: the destructive confirmation (Cancel first); then the status says to remove the extension in Chrome.
 * Depends on: Controls, useModalFocus, useChromeControl, chromeRows, the typed core client.
 * Called by: YourChromeSettings.
 */
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
              <h2 id="chrome-setup-title">Set up Chrome Control?</h2>
            </header>
            <section>
              <p>
                Vibe will be able to work in your own Chrome, in tabs you pick, while you watch.
              </p>
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
            <section className="skills-actions">
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
            </section>
          </>
        ) : (
          <>
            <header>
              <h2 id="chrome-setup-title">Add the extension in Chrome</h2>
            </header>
            <section>
              <p>
                In the Chrome Web Store page, click Add to Chrome. Then open the extension and pair
                with Vibe Work once. Compare the identity below with the one Chrome shows.
              </p>
              <p className="key-line" role="status" aria-live="polite" data-chrome-setup-status="">
                {setupLine(view)}
              </p>
              {chrome.status?.alert === true ? (
                <p className="key-problem" role="alert">
                  {chrome.status.text}
                </p>
              ) : null}
            </section>
            <section className="skills-actions">
              <Button
                data-testid="settings.chrome-setup-store"
                data-dialog-initial-focus="true"
                disabled={busy}
                onClick={() => void run({ op: "openStore" }, "openStore")}
              >
                Open the store page again
              </Button>
              <Button data-testid="settings.chrome-setup-close" onClick={onClose}>
                Close
              </Button>
            </section>
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
          <h2 id="chrome-remove-title">Remove Chrome Control?</h2>
        </header>
        <section>
          <p>
            Removes the helper from Vibe Work's folder and from Chrome's settings, turns its tools
            off and deletes its saved level. Your Chrome, its tabs and the extension stay: remove
            the extension in Chrome.
          </p>
        </section>
        <section className="skills-actions">
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
        </section>
      </div>
    </>
  );
}
