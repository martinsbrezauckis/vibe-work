/** Shared browser removal confirmation. Uses the confirmed core view and command hook; called by connection controls. */
import { TEXT } from "../../../vocabulary";
import { type ReactElement, useRef } from "react";

import { Button } from "../../../components/Controls";
import { useModalFocus } from "../../../hooks/useModalFocus";
import type { ChromeControlState } from "./useChromeControl";

/** The Remove dialog. */
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
  const prefix = browser === "Safari" ? "safari" : "chrome";
  const ref = useRef<HTMLDivElement>(null);
  useModalFocus(open, ref, onClose);
  if (!open) return null;
  return (
    <>
      <div className="scrim" data-testid={`settings.${prefix}-remove-scrim`} onClick={onClose} />
      <div
        className="modal"
        data-testid={`settings.${prefix}-remove-dialog`}
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={`${prefix}-remove-title`}
        tabIndex={-1}
      >
        <header>
          <h2 id={`${prefix}-remove-title`}>
            {browser === "Safari"
              ? TEXT.safariRemoveQuestion
              : chrome.view?.firefox !== undefined
                ? TEXT.allBrowsersRemoveQuestion
                : chrome.view?.edge === undefined
                  ? TEXT.chromeRemoveQuestion
                  : TEXT.browsersRemoveQuestion}
          </h2>
        </header>
        <section>
          <p>
            {browser === "Safari"
              ? TEXT.safariRemoveEffect
              : chrome.view?.firefox !== undefined
                ? TEXT.allBrowsersRemoveEffect
                : chrome.view?.edge === undefined
                  ? ["external", "restartChrome"].includes(chrome.view?.setup.installState ?? "")
                    ? TEXT.chromeRemoveExternalEffect
                    : TEXT.chromeRemoveEffect
                  : TEXT.browsersRemoveEffect}
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
      </div>
    </>
  );
}
