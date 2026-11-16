/**
 * The composer's text preview dialog, shared by a document's extracted text and a long paste: a title, the fact
 * lines, the text itself as plain text, optional actions, and Close last (the initial focus).
 * Depends on: components/Modal, Controls. Called by: DocPreview, PasteChips.
 * Invariants: the text is untrusted (a user file's OCR output or pasted text) and is shown as a text node in `<pre>`,
 * never as Markdown or HTML. Test ids: `composer.<id>-scrim`, `composer.<id>-modal`, `<id>-text`, the Close button's
 * `composer.<close.testId>`; title `<id>-title`.
 */
import type { ReactElement, ReactNode } from "react";

import { Button } from "../../components/Controls";
import { Modal } from "../../components/Modal";

/** The text preview dialog: `title`, `facts`, the text as plain text, then `actions` and the Close button. */
export function TextPreview({
  id,
  title,
  facts,
  text,
  after,
  actions,
  close,
  onClose,
}: {
  id: "doc-preview" | "paste-preview";
  title: string;
  /** The lines above the text. */
  facts: ReactNode;
  text: string;
  /** Below the text, inside the dialog's body (e.g. "Show all"). */
  after?: ReactNode;
  /** Buttons before Close. */
  actions?: ReactNode;
  /** The Close button: its test id (`composer.<testId>`) and accessible name. */
  close: { testId: "doc-preview-btn-primary" | "paste-preview-close"; label: string };
  onClose: () => void;
}): ReactElement {
  return (
    <Modal
      onClose={onClose}
      scrimTestId={`composer.${id}-scrim`}
      data-testid={`composer.${id}-modal`}
      className="modal doc-preview"
      aria-labelledby={`${id}-title`}
    >
      <header>
        <h2 id={`${id}-title`}>{title}</h2>
      </header>
      <section>
        {facts}
        <pre className="out plain doc-text" data-testid={`${id}-text`}>
          {text}
        </pre>
        {after}
      </section>
      <footer className="modal-actions">
        {actions}
        <Button
          data-testid={`composer.${close.testId}`}
          type="button"
          appearance="dialog"
          title={close.label}
          aria-label={close.label}
          data-dialog-initial-focus="true"
          onClick={onClose}
        >
          Close
        </Button>
      </footer>
    </Modal>
  );
}
