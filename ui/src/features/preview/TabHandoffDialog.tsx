/** Reviews the exact user page before delegating it; its rectangle reserves native webview space. */
import { Button } from "../../components/Controls";
import { useLayoutEffect, useRef, type ReactElement } from "react";
import { createPortal } from "react-dom";

import { reserve } from "../../components/nativeLayer";
import { useModalFocus } from "../../hooks/useModalFocus";
import type { HandoffReview } from "./browser";

export function TabHandoffDialog({
  review,
  busy,
  error,
  onClose,
  onConfirm,
}: {
  review: HandoffReview;
  busy: boolean;
  error: string | null;
  onClose: () => void;
  onConfirm: () => void;
}): ReactElement {
  const dialog = useRef<HTMLDivElement>(null);
  useModalFocus(true, dialog, busy ? () => undefined : onClose);
  useLayoutEffect(() => {
    const element = dialog.current;
    if (!element) return;
    const measure = () => {
      const rect = element.getBoundingClientRect();
      reserve(
        "browser-tab-handoff",
        rect.width > 0 && rect.height > 0
          ? { x: rect.left, y: rect.top, width: rect.width, height: rect.height }
          : null,
      );
    };
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    window.addEventListener("resize", measure);
    measure();
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measure);
      reserve("browser-tab-handoff", null);
    };
  }, []);
  return createPortal(
    <>
      <div
        data-testid="preview.tab-handoff-dialog-scrim"
        className="scrim"
        onClick={busy ? undefined : onClose}
      />
      <div
        data-testid="preview.tab-handoff-dialog-modal"
        className="modal browser-handoff-dialog"
        ref={dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="browser-handoff-title"
        data-native-overlay="browser-tab-handoff"
        tabIndex={-1}
      >
        <header>
          <h2 id="browser-handoff-title">Give this tab to the agent?</h2>
        </header>
        <section>
          <p>The agent can read and act on this page after you give it control.</p>
          <p>
            <strong>{review.site || "This page"}</strong>
            <br />
            <span className="browser-handoff-url">{review.url || "Blank tab"}</span>
          </p>
          <p>This does not give the agent access to your vault.</p>
          {error && <p role="alert">{error}</p>}
        </section>
        <footer className="modal-actions">
          <Button
            data-testid="preview.tab-handoff-dialog-tb-btn"
            type="button"

            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </Button>
          <Button
            data-testid="preview.tab-handoff-dialog-btn-primary"
            type="button"
            appearance="dialog"
            data-dialog-initial-focus
            disabled={busy}
            onClick={onConfirm}
          >
            {busy ? "Giving control…" : "Give control"}
          </Button>
        </footer>
      </div>
    </>,
    document.body,
  );
}
