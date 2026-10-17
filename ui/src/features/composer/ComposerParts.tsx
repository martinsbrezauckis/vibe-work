/**
 * Small composer pieces: the run bar (working, Stop, how Enter and Tab act), the list of
 * queued turns above the composer (edit or remove until they start), the chips for inserted context and
 * attached images, and the Retry row after an interrupted or failed turn.
 * Depends on: the composer store hooks, the composer model, Icon. Called by: Composer.
 * Invariants: render only; every button calls one composer action. Context chips show their label and
 * source only, never as editable text. A read document shows its facts and a Preview
 * button instead of its text.
 */
import type { ReactElement } from "react";

import { Icon } from "../../components/Icon";
import { Walker } from "../../components/Walker";
import type { ContextItem, ImageItem } from "./model/message";
import type { QueuedTurn } from "./model/session";

/** The bar shown while a turn runs. */
export function RunBar({ status, onStop }: { status: string; onStop: () => void }): ReactElement {
  return (
    <div className="runbar" role="status">
      <Walker />
      <span className="runtext">
        {status || "Working."} Enter steers, Tab queues the next prompt.
      </span>
      <button
        data-testid="composer.composer-parts-mini"
        type="button"
        className="mini stop"
        title="Stop the running turn"
        aria-label="Stop the running turn"
        onClick={onStop}
      >
        Stop
      </button>
    </div>
  );
}

/** The queued turns, oldest first, each editable and removable until it starts. */
export function QueueList({
  items,
  editing,
  onEdit,
  onChange,
  onSave,
  onCancel,
  onRemove,
}: {
  items: readonly QueuedTurn[];
  editing: { queueItemId: string; text: string } | null;
  onEdit: (item: QueuedTurn) => void;
  onChange: (text: string) => void;
  onSave: () => void;
  onCancel: () => void;
  onRemove: (id: string) => void;
}): ReactElement | null {
  if (items.length === 0) return null;
  return (
    <ol className="queue" aria-label="Queued messages">
      {items.map((item, i) => (
        <li data-item-id={item.id} key={item.id} className="queue-item" data-queue-id={item.id}>
          <span className="queue-n">{String(i + 1)}</span>
          {editing?.queueItemId === item.id ? (
            <>
              <input
                data-testid="composer.composer-parts-on-change"
                className="queue-edit"
                aria-label="Edit the queued message"
                title="Edit the queued message"
                value={editing.text}
                onChange={(e) => {
                  onChange(e.target.value);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") onSave();
                  if (e.key === "Escape") onCancel();
                }}
              />
              <button
                data-testid="composer.composer-parts-mini-2"
                type="button"
                className="mini"
                title="Save queued message (Enter)"
                aria-label="Save queued message (Enter)"
                onClick={onSave}
              >
                Save
              </button>
              <button
                data-testid="composer.composer-parts-mini-3"
                type="button"
                className="mini"
                title="Cancel queued message edit (Escape)"
                aria-label="Cancel queued message edit (Escape)"
                onClick={onCancel}
              >
                Cancel
              </button>
            </>
          ) : (
            <>
              <span className="queue-text" title={item.text}>
                {item.text || "(no text)"}
                {item.extras > 0 ? (
                  <span className="queue-extra"> +{String(item.extras)}</span>
                ) : null}
              </span>
              <button
                data-testid="composer.composer-parts-on-edit"
                type="button"
                className="mini"
                title="Edit queued message"
                aria-label="Edit queued message"
                onClick={() => {
                  onEdit(item);
                }}
              >
                Edit
              </button>
              <button
                data-testid="composer.composer-parts-on-remove"
                type="button"
                className="ibtn sm"
                aria-label="Remove the queued message"
                title="Remove the queued message"
                onClick={() => {
                  onRemove(item.id);
                }}
              >
                <Icon name="x" />
              </button>
            </>
          )}
        </li>
      ))}
    </ol>
  );
}

/** The facts line of a read document's chip: "PDF · 3 pages · 4,210 characters". */
export function docFacts(item: ContextItem): string {
  const doc = item.doc;
  if (doc === undefined) return "";
  const pages =
    doc.pages === null ? null : `${String(doc.pages)} page${doc.pages === 1 ? "" : "s"}`;
  return [
    doc.kindLabel,
    pages,
    doc.chars === 0 ? null : `${doc.chars.toLocaleString("en")} characters`,
  ]
    .filter((part) => part !== null)
    .join(" · ");
}

/** A read document as a chip: name, facts, "cut" when it was shortened, Preview. */
function DocContextChip({
  item,
  onRemove,
  onPreview,
}: {
  item: ContextItem;
  onRemove: () => void;
  onPreview: () => void;
}): ReactElement {
  return (
    <span
      data-item-id={item.id}
      className="ctxchip docctx"
      data-context-id={item.id}
      data-doc-uri={item.uri}
    >
      <Icon name="file-text" size={14} />
      <b>{item.label}</b>
      <span className="mono">
        {docFacts(item)}
        {item.doc?.kept === null ? "" : " · shortened"}
      </span>
      <button
        data-testid="composer.composer-parts-mini-4"
        type="button"
        className="mini"
        title={`Preview text from ${item.label}`}
        aria-label={`Preview text from ${item.label}`}
        onClick={onPreview}
      >
        Preview
      </button>
      <button
        data-testid="composer.composer-parts-button"
        type="button"
        aria-label={`Remove ${item.label}`}
        title={`Remove ${item.label}`}
        onClick={onRemove}
      >
        <Icon name="x" />
      </button>
    </span>
  );
}

/** Inserted context, read documents and attached images, as removable chips. */
export function ChipsRow({
  context,
  images,
  onRemoveContext,
  onRemoveImage,
  onPreview,
}: {
  context: readonly ContextItem[];
  images: readonly ImageItem[];
  onRemoveContext: (id: string) => void;
  onRemoveImage: (id: string) => void;
  /** Opens the preview of a read document's chip. */
  onPreview?: (id: string) => void;
}): ReactElement | null {
  if (context.length === 0 && images.length === 0) return null;
  return (
    <div className="ctx-row" aria-label="Sent with the next message">
      {context.map((item) =>
        item.doc === undefined ? (
          <span
            data-item-id={item.id}
            key={item.id}
            className="ctxchip"
            data-context-id={item.id}
            title={item.text}
          >
            <b>{item.label}</b>
            <span className="mono">{item.text}</span>
            <button
              data-testid="composer.composer-parts-on-remove-context"
              type="button"
              aria-label={`Remove ${item.label}`}
              title={`Remove ${item.label}`}
              onClick={() => {
                onRemoveContext(item.id);
              }}
            >
              <Icon name="x" />
            </button>
          </span>
        ) : (
          <DocContextChip
            key={item.id}
            item={item}
            onRemove={() => {
              onRemoveContext(item.id);
            }}
            onPreview={() => {
              onPreview?.(item.id);
            }}
          />
        ),
      )}
      {images.map((image) => (
        <span data-item-id={image.id} key={image.id} className="att" data-image-id={image.id}>
          <img src={`data:${image.mimeType};base64,${image.data}`} alt="" className="att-img" />
          <span className="att-name">{image.name}</span>
          <button
            data-testid="composer.composer-parts-on-remove-image"
            type="button"
            aria-label={`Remove ${image.name}`}
            title={`Remove ${image.name}`}
            onClick={() => {
              onRemoveImage(image.id);
            }}
          >
            <Icon name="x" />
          </button>
        </span>
      ))}
    </div>
  );
}

/** Offered after a turn that was stopped or failed. */
export function RetryRow({
  problem,
  disabled = false,
  onRetry,
}: {
  problem: "interrupted" | "failed";
  disabled?: boolean;
  onRetry: () => void;
}): ReactElement {
  return (
    <div className="retry-row">
      <span>
        {problem === "interrupted" ? "The last turn was stopped." : "The last turn failed."}
      </span>
      <button
        data-testid="composer.composer-parts-mini-5"
        type="button"
        className="mini"
        disabled={disabled}
        title="Retry the last stopped or failed turn"
        aria-label="Retry the last stopped or failed turn"
        onClick={onRetry}
      >
        <Icon name="rotate-ccw" /> Retry
      </button>
    </div>
  );
}
