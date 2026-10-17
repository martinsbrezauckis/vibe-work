/**
 * Documents waiting in a session's composer: one chip per document not yet read, with its
 * kind, page count and the quota sentence ("Reading it uses 3 pages of your Mistral OCR quota"), and the
 * buttons "Read text" (Mistral OCR, through the core), "Attach as picture" (images Vibe can see) and
 * Remove. A failed read shows Mistral's reason and offers "Try again". Read documents leave this row
 * and become context chips (`ChipsRow`).
 * Depends on: AppContext, the documents store, Icon. Called by: Composer.
 * Invariants: render only; nothing is sent to OCR before "Read text" is pressed; texts come from the
 * core (plain text nodes).
 */
import type { ReactElement } from "react";

import { Icon } from "../../components/Icon";
import type { DocumentView } from "../../protocol/core";
import { pagesText, quotaText } from "../../state/documents";
import { useApp } from "../shell/AppContext";
import { KeyNotice, useKeyReadiness } from "../shell/KeyNotice";
import { NONE, useDocumentsState } from "./hooks";

function DocChip({ doc, sessionId }: { doc: DocumentView; sessionId: string }): ReactElement {
  const { documents } = useApp();
  const keyBlocked = useKeyReadiness() !== null;
  const busy = useDocumentsState((s) => s.busy[doc.id] === true || doc.status === "reading");
  if (documents === null) return <></>;
  const actions = documents.actions;
  const failed = doc.status === "failed";
  return (
    <div
      data-item-id={doc.id}
      className={`docchip${failed ? " failed" : ""}`}
      data-doc-id={doc.id}
      data-status={doc.status}
    >
      <div className="docchip-head">
        <Icon name="file-text" size={14} />
        <b className="docchip-name" title={doc.name}>
          {doc.name}
        </b>
        <span className="docchip-meta">
          {doc.kindLabel} · {pagesText(doc.pages)}
        </span>
      </div>
      <p className="docchip-note" role={failed && !keyBlocked ? "alert" : undefined}>
        {failed && !keyBlocked
          ? (doc.error ?? "Reading failed.")
          : busy
            ? "Reading with Mistral OCR…"
            : quotaText(doc)}
      </p>
      <div className="docchip-acts">
        <button
          data-testid="composer.document-chips-extract"
          type="button"
          className="mini"
          disabled={busy || keyBlocked}
          title={`${failed ? "Try again to read text" : "Read text"} from ${doc.name} with Mistral OCR`}
          aria-label={`${failed ? "Try again to read text" : "Read text"} from ${doc.name} with Mistral OCR`}
          onClick={() => void actions.extract(sessionId, doc.id).catch(() => undefined)}
        >
          {failed ? "Try again" : "Read text"}
        </button>
        {doc.canAttachAsPicture ? (
          <button
            data-testid="composer.document-chips-as-picture"
            type="button"
            className="mini"
            disabled={busy}
            title={`Attach ${doc.name} as a picture instead of reading text`}
            aria-label={`Attach ${doc.name} as a picture instead of reading text`}
            onClick={() => void actions.asPicture(sessionId, doc.id).catch(() => undefined)}
          >
            Attach as picture
          </button>
        ) : null}
        <button
          data-testid="composer.document-chips-remove"
          type="button"
          className="ibtn sm"
          aria-label={`Remove ${doc.name}`}
          title={`Remove ${doc.name}`}
          disabled={busy}
          onClick={() => void actions.remove(sessionId, doc.id)}
        >
          <Icon name="x" />
        </button>
      </div>
    </div>
  );
}

/** The documents waiting in `sessionId`'s composer. */
export function DocumentChips({ sessionId }: { sessionId: string }): ReactElement | null {
  const { documents } = useApp();
  const chips = useDocumentsState((s) => s.chips[sessionId] ?? NONE);
  if (documents === null || chips.length === 0) return null;
  return (
    <div className="doc-row" aria-label="Attached files with optional text reading">
      <KeyNotice message="Reading document text uses your Mistral key for OCR, which is kept in the vault." />
      {chips.map((doc) => (
        <DocChip key={doc.id} doc={doc} sessionId={sessionId} />
      ))}
    </div>
  );
}
