/** Shared browser removal confirmation. Uses the confirmed core view and command hook; called by connection controls. */
import { TEXT, wording } from "../../../vocabulary";
import type { ReactElement } from "react";

import { Button } from "../../../components/Controls";
import { Modal } from "../../../components/Modal";
import type { ChromeControlState } from "./useChromeControl";

/**
 * The browser card's Remove dialog. Its text says what happens to the extension: when Vibe Work installed it
 * through its own browser registration (`installState` external or restartChrome), Remove undoes that registration
 * and Chrome takes the extension out of all its profiles by itself; otherwise the extension stays in the browser.
 */
export function ChromeRemoveDialog({
  open,
  chrome,
  browser = "Chrome",
  onClose,
}: {
  open: boolean;
  chrome: ChromeControlState;
  browser?: "Chrome" | "Edge" | "Safari" | "Firefox";
  onClose: () => void;
}): ReactElement | null {
  const prefix = browser === "Firefox" ? "firefox" : browser === "Safari" ? "safari" : "chrome";
  if (!open) return null;
  return (
    <>
      <Modal
        onClose={onClose}
        scrimTestId={`settings.${prefix}-remove-scrim`}
        data-testid={`settings.${prefix}-remove-dialog`}
        aria-labelledby={`${prefix}-remove-title`}
      >
        <header>
          <h2 id={`${prefix}-remove-title`}>
            {TEXT.browserRemoveQuestion.replace("{browser}", browser)}
          </h2>
        </header>
        <section>
          <p>
            {browser === "Firefox"
              ? "Closes the Vibe Work Firefox window and removes its connection. Firefox profiles and the add-on stay in Firefox."
              : browser === "Safari"
                ? TEXT.safariRemoveEffect
                : browser === "Chrome" &&
                    ["external", "restartChrome"].includes(chrome.view?.setup.installState ?? "")
                  ? TEXT.chromeRemoveExternalEffect
                  : TEXT.browserRemoveEffect.replaceAll("{browser}", browser)}
          </p>
        </section>
        <footer className="modal-actions">
          <Button
            data-testid={`settings.${prefix}-remove-cancel`}
            data-dialog-initial-focus="true"
            onClick={onClose}
          >
            Cancel
          </Button>
          <Button
            data-testid={`settings.${prefix}-remove-confirm`}
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
      </Modal>
    </>
  );
}

/**
 * Asks before a profile row's Remove forgets that profile's pairing. The extension stays on in the browser, so the
 * text says so; the command runs only from the dialog's own Remove.
 */
export function ProfileRemoveDialog({
  profile,
  chrome,
  browser,
  onClose,
}: {
  profile: { profileId: string; name: string } | null;
  chrome: ChromeControlState;
  browser: string;
  onClose: () => void;
}): ReactElement | null {
  if (profile === null) return null;
  const id = profile.profileId;
  return (
    <>
      <Modal
        onClose={onClose}
        scrimTestId="settings.browser-profile-remove-scrim"
        data-testid="settings.browser-profile-remove-dialog"
        aria-labelledby="browser-profile-remove-title"
        aria-describedby="browser-profile-remove-effect"
      >
        <header>
          <h2 id="browser-profile-remove-title">
            {wording(TEXT.browserProfileRemoveQuestion, { browser, profile: profile.name })}
          </h2>
        </header>
        <section>
          <p id="browser-profile-remove-effect">
            {wording(TEXT.browserProfileRemoveEffect, { browser })}
          </p>
        </section>
        <footer className="modal-actions">
          <Button
            data-testid="settings.browser-profile-remove-cancel"
            data-dialog-initial-focus="true"
            onClick={onClose}
          >
            Cancel
          </Button>
          <Button
            data-testid="settings.browser-profile-remove-confirm"
            data-item-id={id}
            variant="danger"
            disabled={chrome.busy !== null}
            onClick={() => {
              void chrome.run({ op: "removeProfile", profileId: id }, id).then((result) => {
                if (result !== null) onClose();
              });
            }}
          >
            Remove
          </Button>
        </footer>
      </Modal>
    </>
  );
}
