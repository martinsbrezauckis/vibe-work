/**
 * Names an opaque browser choice while keeping the native page visible beside the prompt. The measured dialog
 * reserves its screen rectangle before the native viewport may appear, so the page cannot paint over the field.
 * Depends on: modal focus, native-layer reservations and the browser choice metadata. Called by: SaveVault.
 */
import { type ReactElement, type SubmitEvent, useLayoutEffect, useRef, useState } from "react";

import { reserve } from "../../components/nativeLayer";
import { useModalFocus } from "../../hooks/useModalFocus";
import type { VaultChoice } from "./browser";

const DIALOG_RESERVATION = "save-vault-dialog";

/** The Save choice's name prompt; closing it is owned by the caller so the opaque choice can be cancelled. */
export function SaveNameDialog({
  choice,
  busy,
  onClose,
  onSave,
}: {
  choice: VaultChoice;
  busy: boolean;
  onClose: () => void;
  onSave: (name: string) => void;
}): ReactElement {
  const [name, setName] = useState(choice.suggestedName);
  const dialog = useRef<HTMLDivElement>(null);
  useModalFocus(true, dialog, busy ? () => undefined : onClose);
  useLayoutEffect(() => {
    const element = dialog.current;
    if (element === null) return;
    const measure = () => {
      const rect = element.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) {
        delete element.dataset.nativeReady;
        reserve(DIALOG_RESERVATION, null);
        return;
      }
      reserve(DIALOG_RESERVATION, {
        x: rect.left,
        y: rect.top,
        width: rect.width,
        height: rect.height,
      });
      element.dataset.nativeReady = "true";
    };
    const resize = new ResizeObserver(measure);
    resize.observe(element);
    window.addEventListener("resize", measure);
    document.addEventListener("scroll", measure, true);
    measure();
    return () => {
      delete element.dataset.nativeReady;
      resize.disconnect();
      window.removeEventListener("resize", measure);
      document.removeEventListener("scroll", measure, true);
      reserve(DIALOG_RESERVATION, null);
    };
  }, []);
  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (name.trim() !== "" && !busy) onSave(name.trim());
  };
  return (
    <>
      <div
        data-testid="preview.save-name-dialog-scrim"
        className="scrim"
        onClick={busy ? undefined : onClose}
      />
      <div
        data-testid="preview.save-name-dialog-modal"
        className="modal preview-save-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="preview-save-title"
        data-native-overlay="preview-save"
        ref={dialog}
        tabIndex={-1}
      >
        <form onSubmit={submit}>
          <header>
            <h2 id="preview-save-title">Save to vault</h2>
          </header>
          <section>
            <label className="preview-save-name">
              <span>Name</span>
              <input
                data-testid="preview.save-name-dialog-set-name"
                data-vault-save-name
                data-dialog-initial-focus="true"
                value={name}
                maxLength={80}
                autoComplete="off"
                onChange={(event) => {
                  setName(event.target.value);
                }}
              />
            </label>
            <p className="preview-save-hint">The selected value goes straight to your vault.</p>
          </section>
          <footer className="modal-actions">
            <button
              data-testid="preview.save-name-dialog-tb-btn"
              type="button"
              className="tb-btn"
              disabled={busy}
              onClick={onClose}
            >
              Cancel
            </button>
            <button
              data-testid="preview.save-name-dialog-btn-primary"
              type="submit"
              className="btn-primary"
              data-vault-save-confirm
              disabled={busy || name.trim() === ""}
            >
              {busy ? "Saving…" : "Save to vault"}
            </button>
          </footer>
        </form>
      </div>
    </>
  );
}
