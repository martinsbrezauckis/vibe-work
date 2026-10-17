/**
 * The preview of a document's locally extracted or OCR text, as
 * plain text (the first 20,000 characters, "Show all" for the rest), with the facts: kind, pages,
 * characters, the OCR model that read it, and what was kept of a long document.
 * Depends on: AppContext, the documents and composer stores, useModalFocus, model/text. Called by: Composer.
 * Invariants: render only; the text is untrusted (OCR output of a user file) and is shown as a text
 * node in `<pre>`, never as Markdown or HTML.
 */
import { type ReactElement, useRef, useState } from "react";

import { useModalFocus } from "../../hooks/useModalFocus";
import { keptText, pagesText } from "../../state/documents";
import { safeHead } from "../transcript/model/text";
import { useApp } from "../shell/AppContext";
import { useComposerState, useDocumentsState } from "./hooks";

/** Characters shown before "Show all". */
export const PREVIEW_CHARS = 20_000;

/** The preview dialog, while one is open. */
export function DocPreview(): ReactElement | null {
  const { documents } = useApp();
  const target = useDocumentsState((s) => s.preview);
  const item = useComposerState((s) =>
    target === null
      ? undefined
      : (s.context[target.sessionId] ?? []).find((c) => c.id === target.contextId),
  );
  const [all, setAll] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const open = target !== null && item !== undefined;
  const close = () => {
    setAll(false);
    documents?.actions.preview(null);
  };
  useModalFocus(open, ref, close);
  if (!open) return null;
  const doc = item.doc;
  const long = item.text.length > PREVIEW_CHARS;
  const kept = doc === undefined ? null : keptText(doc.kept);
  return (
    <>
      <div data-testid="composer.doc-preview-scrim" className="scrim" onClick={close} />
      <div
        data-testid="composer.doc-preview-modal"
        ref={ref}
        className="modal doc-preview"
        role="dialog"
        aria-modal="true"
        aria-labelledby="doc-preview-title"
        tabIndex={-1}
      >
        <header>
          <h2 id="doc-preview-title">{item.label}</h2>
        </header>
        <section>
          {doc === undefined ? null : (
            <p className="d">
              {doc.kindLabel} · {doc.pages === null ? "not paged" : pagesText(doc.pages)} ·{" "}
              {doc.chars.toLocaleString("en")} characters ·{" "}
              {doc.model === null ? "read on this computer" : `read by ${doc.model}`}
            </p>
          )}
          {kept === null ? null : <p className="doc-kept">{kept}</p>}
          <p className="d">
            The saved file path is sent with your message. Short extracted text is included too.
          </p>
          <pre className="out plain doc-text" data-testid="doc-preview-text">
            {all || !long ? item.text : safeHead(item.text, PREVIEW_CHARS)}
          </pre>
          {long && !all ? (
            <button
              data-testid="composer.doc-preview-set-all"
              type="button"
              className="show-all-btn"
              title="Show all document text"
              aria-label="Show all document text"
              onClick={() => {
                setAll(true);
              }}
            >
              Show all ({Math.round((item.text.length - PREVIEW_CHARS) / 1000).toLocaleString("en")}{" "}
              KB more)
            </button>
          ) : null}
        </section>
        <footer className="modal-actions">
          <button
            data-testid="composer.doc-preview-btn-primary"
            type="button"
            className="btn-primary"
            title="Close document preview (Escape)"
            aria-label="Close document preview (Escape)"
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
