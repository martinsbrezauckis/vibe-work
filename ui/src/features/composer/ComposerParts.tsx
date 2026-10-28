/**
 * Small composer pieces: the run bar (working, Stop, how Enter and Tab act), the list of
 * queued turns above the composer (edit or remove until they start), the chips for inserted context and
 * attached images, and the Retry row after an interrupted or failed turn.
 * Depends on: the composer store hooks, the composer model, Icon. Called by: Composer.
 * Invariants: render only; every button calls one composer action. Context chips show their label and
 * source only, never as editable text. A read document shows its facts and a Preview
 * button instead of its text.
 */
import { Badge, Button } from "../../components/Controls";
import type { ReactElement } from "react";

import { Icon } from "../../components/Icon";
import { Walker } from "../../components/Walker";
import type { ContextItem, ImageItem } from "./model/message";
import type { QueuedTurn } from "./model/session";
import type { CodeRef } from "../files/model/context";

/** The bar shown while a turn runs. */
export function RunBar({
  status,
  waiting = false,
  inputWaiting = false,
  onStop,
  onShowPrompt,
}: {
  status: string;
  waiting?: boolean;
  inputWaiting?: boolean;
  onStop: () => void;
  /** While a prompt waits, the status is a button that goes to it (the next one on each press). */
  onShowPrompt?: () => void;
}): ReactElement {
  return (
    <div className="runbar" role="status">
      {waiting ? <span className="px warn" aria-hidden="true" /> : <Walker />}
      <span className="runtext">
        {waiting && onShowPrompt !== undefined ? (
          <Button
            data-testid="composer.run-bar-show-prompt"
            appearance="text-link"
            onClick={onShowPrompt}
          >
            {status}
          </Button>
        ) : (
          status || "Working."
        )}
        {inputWaiting
          ? " Enter or Tab sends your message now."
          : waiting
            ? ""
            : " Enter steers, Tab queues the next prompt."}
      </span>
      <Button
        data-testid="composer.composer-parts-mini"
        type="button"
        appearance="compact"
        placement="stop"
        title="Stop the running turn"
        aria-label="Stop the running turn"
        onClick={onStop}
      >
        Stop
      </Button>
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
              <Button
                data-testid="composer.composer-parts-mini-2"
                type="button"
                appearance="compact"
                title="Save queued message (Enter)"
                aria-label="Save queued message (Enter)"
                onClick={onSave}
              >
                Save
              </Button>
              <Button
                data-testid="composer.composer-parts-mini-3"
                type="button"
                appearance="compact"
                title="Cancel queued message edit (Escape)"
                aria-label="Cancel queued message edit (Escape)"
                onClick={onCancel}
              >
                Cancel
              </Button>
            </>
          ) : (
            <>
              <span className="queue-text" title={item.text}>
                {item.text || "(no text)"}
                {item.extras > 0 ? (
                  <span className="queue-extra"> +{String(item.extras)}</span>
                ) : null}
              </span>
              <Button
                data-testid="composer.composer-parts-on-edit"
                type="button"
                appearance="compact"
                title="Edit queued message"
                aria-label="Edit queued message"
                onClick={() => {
                  onEdit(item);
                }}
              >
                Edit
              </Button>
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
function docFacts(item: ContextItem): string {
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
    <Badge
      data-item-id={item.id}
      presentation="document-context"
      data-context-id={item.id}
      data-doc-uri={item.uri}
    >
      <Icon name="file-text" size={14} />
      <b>{item.label}</b>
      <span className="mono">
        {docFacts(item)}
        {item.doc?.kept === null ? "" : " · shortened"}
      </span>
      <Button
        data-testid="composer.composer-parts-mini-4"
        type="button"
        appearance="compact"
        title={`Preview text from ${item.label}`}
        aria-label={`Preview text from ${item.label}`}
        onClick={onPreview}
      >
        Preview
      </Button>
      <button
        data-testid="composer.composer-parts-button"
        type="button"
        aria-label={`Remove ${item.label}`}
        title={`Remove ${item.label}`}
        onClick={onRemove}
      >
        <Icon name="x" />
      </button>
    </Badge>
  );
}

/** Inserted context, read documents and attached images, as removable chips. */
export function ChipsRow({
  context,
  images,
  onRemoveContext,
  onRemoveImage,
  onPreview,
  onOpenRef,
}: {
  context: readonly ContextItem[];
  images: readonly ImageItem[];
  onRemoveContext: (id: string) => void;
  onRemoveImage: (id: string) => void;
  /** Opens the preview of a read document's chip. */
  onPreview?: (id: string) => void;
  /** Opens a code chip's absolute file and range. */
  onOpenRef?: (ref: CodeRef) => void;
}): ReactElement | null {
  if (context.length === 0 && images.length === 0) return null;
  return (
    <div className="ctx-row" aria-label="Sent with the next message">
      {context.map((item) =>
        item.doc === undefined ? (
          <Badge
            data-item-id={item.id}
            key={item.id}
            presentation="context"
            data-context-id={item.id}
            title={item.text}
          >
            {item.ref === undefined ? (
              <b>{item.label}</b>
            ) : (
              <Button
                data-testid="composer.composer-parts-open-context"
                type="button"
                appearance="file-reference"
                title={`Open ${item.label} in the File tab`}
                aria-label={`Open ${item.label} in the File tab`}
                onClick={() => {
                  if (item.ref !== undefined) onOpenRef?.(item.ref);
                }}
              >
                <b>{item.label}</b>
              </Button>
            )}
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
          </Badge>
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
      <Button
        data-testid="composer.composer-parts-mini-5"
        type="button"
        appearance="compact"
        disabled={disabled}
        title="Retry the last stopped or failed turn"
        aria-label="Retry the last stopped or failed turn"
        onClick={onRetry}
      >
        <Icon name="rotate-ccw" /> Retry
      </Button>
    </div>
  );
}
