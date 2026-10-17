/**
 * A document's text as Mistral extracted it: what a source of an answer, or "Open text", shows. A modal
 * with the text in a scrolling block.
 * Depends on: useModalFocus, hooks. Called by: LibrariesPage.
 * Invariants: the text is file content (possibly hostile): rendered as plain text in a `<pre>`, never as HTML or
 * Markdown.
 */
import { type ReactElement, useCallback, useRef } from "react";

import { useModalFocus } from "../../hooks/useModalFocus";
import { useLibraries, useLibraryActions } from "./hooks";

/** The viewer (rendered while a text is open). */
export function TextViewer(): ReactElement | null {
  const actions = useLibraryActions();
  const viewer = useLibraries((s) => s.viewer);
  const dialog = useRef<HTMLDivElement>(null);
  const close = useCallback(() => actions?.closeViewer(), [actions]);
  useModalFocus(viewer !== null, dialog, close);
  if (viewer === null) return null;
  return (
    <>
      <div data-testid="libraries.text-viewer-scrim" className="scrim" onClick={close} />
      <div
        className="modal lib-viewer"
        role="dialog"
        aria-modal="true"
        aria-labelledby="lib-viewer-title"
        ref={dialog}
        tabIndex={-1}
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
          <button
            data-testid="libraries.text-viewer-tb-btn"
            type="button"
            className="tb-btn"
            data-dialog-initial-focus="true"
            onClick={close}
          >
            Close
          </button>
        </footer>
      </div>
    </>
  );
}
