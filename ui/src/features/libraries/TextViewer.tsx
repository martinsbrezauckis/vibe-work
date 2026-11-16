/**
 * A document's text as Mistral extracted it: what a source of an answer, or "Open text", shows. A modal
 * with the text in a scrolling block.
 * Depends on: Modal, hooks. Called by: LibrariesPage.
 * Invariants: the text is file content (possibly hostile): rendered as plain text in a `<pre>`, never as HTML or
 * Markdown.
 */
import { Button } from "../../components/Controls";
import { type ReactElement, useCallback } from "react";

import { Modal } from "../../components/Modal";
import { useLibraries, useLibraryActions } from "./hooks";

/** The viewer (rendered while a text is open). */
export function TextViewer(): ReactElement | null {
  const actions = useLibraryActions();
  const viewer = useLibraries((s) => s.viewer);
  const close = useCallback(() => actions?.closeViewer(), [actions]);
  if (viewer === null) return null;
  return (
    <Modal
      onClose={close}
      scrimTestId="libraries.text-viewer-scrim"
      className="modal lib-viewer"
      aria-labelledby="lib-viewer-title"
      data-testid="libraries.library-text"
    >
      <header>
        <h2 id="lib-viewer-title">
          {viewer.source === null ? "" : `[${String(viewer.source)}] `}
          {viewer.name}
        </h2>
      </header>
      <section>
        <pre className="lib-text">{viewer.text}</pre>
        {viewer.truncated ? (
          <p className="cmuted">The text is longer; this shows the start.</p>
        ) : null}
      </section>
      <footer>
        <Button
          data-testid="libraries.text-viewer-tb-btn"
          type="button"

          data-dialog-initial-focus="true"
          onClick={close}
        >
          Close
        </Button>
      </footer>
    </Modal>
  );
}
