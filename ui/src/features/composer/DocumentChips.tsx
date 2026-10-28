/** Automatic attachment reading and retry controls. Uses the documents store; render only. */
import { Button } from "../../components/Controls";
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
      <p className="docchip-note" role={failed ? "alert" : undefined}>
        {failed
          ? (doc.error ?? "Reading failed.")
          : busy
            ? "Reading with Mistral OCR…"
            : quotaText(doc)}
      </p>
      <div className="docchip-acts">
        {failed ? (
          <Button
            data-testid="composer.document-chips-extract"
            type="button"
            appearance="compact"
            disabled={busy || keyBlocked}
            title={`Try again to read text from ${doc.name}`}
            aria-label={`Try again to read text from ${doc.name}`}
            onClick={() => void actions.extract(sessionId, doc.id).catch(() => undefined)}
          >
            Try again
          </Button>
        ) : null}
        {doc.canAttachAsPicture ? (
          <Button
            data-testid="composer.document-chips-as-picture"
            type="button"
            appearance="compact"
            disabled={busy}
            title={`Attach ${doc.name} as a picture instead of reading text`}
            aria-label={`Attach ${doc.name} as a picture instead of reading text`}
            onClick={() => void actions.asPicture(sessionId, doc.id)}
          >
            Attach as picture
          </Button>
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
    <div className="doc-row" aria-label="Reading attached files">
      <KeyNotice message="Reading document text uses your Mistral key for OCR, which is kept in the vault." />
      {chips.map((doc) => (
        <DocChip key={doc.id} doc={doc} sessionId={sessionId} />
      ))}
    </div>
  );
}
