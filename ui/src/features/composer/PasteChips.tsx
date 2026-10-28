/**
 * Long pastes in the message box (`model/pastes.ts`): `usePasteFold` turns a paste over the threshold into a
 * marker in the draft and makes one Backspace (or Delete) remove a whole marker; `PasteChips` shows each
 * marker's paste as a chip ("Pasted text, +42 lines") with Open and Remove; Open shows the full text with
 * "Put in the message box", which swaps the marker for the text.
 * Depends on: AppContext, the composer store hooks, the composer model, Badge, Button, Icon, useModalFocus.
 * Called by: Composer.
 * Invariants: every fold, removal or expansion is one draft change that Ctrl+Z (Cmd+Z) takes back while the box
 * still holds its result; the paste text is shown as a text node, never as HTML.
 */
import {
  type ClipboardEvent,
  type KeyboardEvent,
  type ReactElement,
  type RefObject,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import { Badge } from "../../components/Badge";
import { Button } from "../../components/Controls";
import { Icon } from "../../components/Icon";
import { useModalFocus } from "../../hooks/useModalFocus";
import { pasteIntoDraft } from "../../state/composerPaste";
import { useApp } from "../shell/AppContext";
import { NONE, useComposerState } from "./hooks";
import {
  type PasteItem,
  foldsIntoChip,
  lineCount,
  markerAt,
  normalizePaste,
  pasteLabel,
  pasteMarker,
  pastesIn,
  replaceMarker,
} from "./model/pastes";

/** Paste and key handlers of the message box of `sessionId`. */
export function usePasteFold(sessionId: string, box: RefObject<HTMLTextAreaElement | null>) {
  const { composer } = useApp();
  const caret = useRef<number | null>(null);
  // The last change made here: undone by Ctrl+Z while the box still holds `after`.
  const last = useRef<{ before: string; after: string; caret: number } | null>(null);
  const draft = useComposerState((s) => s.drafts[sessionId] ?? "");
  useLayoutEffect(() => {
    const at = caret.current;
    if (at === null || box.current === null) return;
    caret.current = null;
    box.current.setSelectionRange(at, at);
  }, [box, draft]);

  const change = (after: string, at: number, undoAt: number) => {
    const before = composer.store.get().drafts[sessionId] ?? "";
    last.current = { before, after, caret: undoAt };
    caret.current = at;
    composer.setDraft(sessionId, after);
  };
  const replace = (start: number, end: number, text: string) => {
    const value = composer.store.get().drafts[sessionId] ?? "";
    change(value.slice(0, start) + text + value.slice(end), start + text.length, end);
  };
  /** Replaces the marker of `item` with `by` ("" removes the chip, the full text expands it). */
  const swap = (item: PasteItem, by: string) => {
    const value = composer.store.get().drafts[sessionId] ?? "";
    const at = value.indexOf(pasteMarker(item));
    if (at < 0) return;
    change(replaceMarker(value, item, by), at + by.length, at + pasteMarker(item).length);
  };

  /** Folds a long text paste into a chip; true when it did (the browser's own paste is then skipped). */
  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>): boolean => {
    const text = normalizePaste(e.clipboardData.getData("text/plain"));
    if (!foldsIntoChip(text)) return false;
    e.preventDefault();
    const input = e.currentTarget;
    const result = pasteIntoDraft(
      composer,
      sessionId,
      text,
      input.selectionStart,
      input.selectionEnd,
      "user",
    );
    last.current = { before: result.before, after: result.after, caret: input.selectionEnd };
    caret.current = result.caret;
    return true;
  };

  /**
   * Backspace right after a marker, or Delete right before one, removes the whole marker; Ctrl+Z right after a
   * fold, removal or expansion takes it back.
   */
  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>): boolean => {
    const input = e.currentTarget;
    const undo = last.current;
    if (
      (e.key === "z" || e.key === "Z") &&
      (e.ctrlKey || e.metaKey) &&
      !e.shiftKey &&
      !e.altKey &&
      undo?.after === input.value
    ) {
      e.preventDefault();
      last.current = null;
      caret.current = undo.caret;
      composer.setDraft(sessionId, undo.before);
      return true;
    }
    if (e.key !== "Backspace" && e.key !== "Delete") return false;
    if (input.selectionStart !== input.selectionEnd || e.altKey || e.ctrlKey || e.metaKey) {
      return false;
    }
    const pastes = composer.store.get().pastes[sessionId] ?? NONE;
    const hit = markerAt(input.value, input.selectionStart, pastes, e.key === "Backspace");
    if (hit === null) return false;
    e.preventDefault();
    replace(hit.start, hit.end, "");
    return true;
  };

  return { onPaste, onKey, swap };
}

/** The chips of the long pastes whose marker is in the draft of `sessionId`. */
export function PasteChips({
  sessionId,
  onSwap: swap,
}: {
  sessionId: string;
  /** Replaces a paste's marker in the draft (`usePasteFold().swap`). */
  onSwap: (item: PasteItem, by: string) => void;
}): ReactElement | null {
  const draft = useComposerState((s) => s.drafts[sessionId] ?? "");
  const held = useComposerState((s) => s.pastes[sessionId] ?? NONE);
  const [open, setOpen] = useState<string | null>(null);
  const shown = pastesIn(draft, held);
  if (shown.length === 0) return null;
  const opened = shown.find((item) => item.id === open);
  return (
    <div className="ctx-row" aria-label="Long pastes in this message">
      {shown.map((item) => (
        <Badge
          key={item.id}
          presentation="document-context"
          data-item-id={item.id}
          data-paste-id={item.id}
          title={pasteLabel(item)}
        >
          <Icon name="scroll-text" size={14} />
          <b>Pasted text</b>
          <span className="mono">+{String(lineCount(item.text))} lines</span>
          <Button
            data-testid="composer.paste-chip-open"
            type="button"
            appearance="compact"
            title={`Open pasted text #${String(item.n)}`}
            aria-label={`Open pasted text #${String(item.n)}, ${String(lineCount(item.text))} lines`}
            onClick={() => {
              setOpen(item.id);
            }}
          >
            Open
          </Button>
          <button
            data-testid="composer.paste-chip-remove"
            type="button"
            aria-label={`Remove pasted text #${String(item.n)}`}
            title={`Remove pasted text #${String(item.n)}`}
            onClick={() => {
              swap(item, "");
            }}
          >
            <Icon name="x" />
          </button>
        </Badge>
      ))}
      {opened === undefined ? null : (
        <PastePreview
          item={opened}
          onClose={() => {
            setOpen(null);
          }}
          onExpand={() => {
            setOpen(null);
            swap(opened, opened.text);
          }}
        />
      )}
    </div>
  );
}

/** The full text of one paste, with "Put in the message box". */
function PastePreview({
  item,
  onClose,
  onExpand,
}: {
  item: PasteItem;
  onClose: () => void;
  onExpand: () => void;
}): ReactElement {
  const ref = useRef<HTMLDivElement>(null);
  useModalFocus(true, ref, onClose);
  return (
    <>
      <div data-testid="composer.paste-preview-scrim" className="scrim" onClick={onClose} />
      <div
        data-testid="composer.paste-preview-modal"
        ref={ref}
        className="modal doc-preview"
        role="dialog"
        aria-modal="true"
        aria-labelledby="paste-preview-title"
        tabIndex={-1}
      >
        <header>
          <h2 id="paste-preview-title">Pasted text #{String(item.n)}</h2>
        </header>
        <section>
          <p className="d">
            {String(lineCount(item.text))} lines · {item.text.length.toLocaleString("en")}{" "}
            characters. Sent in full with your message.
          </p>
          <pre className="out plain doc-text" data-testid="paste-preview-text">
            {item.text}
          </pre>
        </section>
        <footer className="modal-actions">
          <Button
            data-testid="composer.paste-preview-expand"
            type="button"
            shape="outlined"
            title="Put the full text in the message box to edit it"
            aria-label="Put the full text in the message box to edit it"
            onClick={onExpand}
          >
            Put in the message box
          </Button>
          <Button
            data-testid="composer.paste-preview-close"
            type="button"
            appearance="dialog"
            title="Close pasted text (Escape)"
            aria-label="Close pasted text (Escape)"
            data-dialog-initial-focus="true"
            onClick={onClose}
          >
            Close
          </Button>
        </footer>
      </div>
    </>
  );
}
