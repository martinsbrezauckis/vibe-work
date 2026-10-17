/**
 * Annotating the app preview (the session's own app only, not web pages): point at an
 * element or draw a box in the page, write a note, collect several, then put them all into the message at once
 * (a context chip and a cropped picture each). After the agent's turn each sent note says whether its element
 * changed. Names, text and routes are page text: shown as text only.
 * Depends on: browser.ts, browserModel.ts. Called by: PreviewTab.
 */
import type { ReactElement } from "react";

import { useStore } from "../../state/createStore";
import type { Browser } from "./browser";
import { type NoteItem, statusText } from "./browserModel";

function NoteRow({
  item,
  onNote,
  onRemove,
}: {
  item: NoteItem;
  onNote: (text: string) => void;
  onRemove: () => void;
}): ReactElement {
  const a = item.annotation;
  const what = a.name !== "" ? `${a.role} “${a.name}”` : a.mode === "box" ? "a box" : a.role;
  return (
    <li data-item-id={item.id} className="browser-note" data-status={item.status}>
      <div className="browser-note-head">
        <span className="browser-note-what">{what}</span>
        <span className="browser-note-route">{a.route}</span>
        <span className="browser-note-status">{statusText(item.status)}</span>
        <button
          data-testid="preview.annotations-preview-btn"
          type="button"
          className="preview-btn"
          aria-label="Remove this note"
          onClick={onRemove}
        >
          Remove
        </button>
      </div>
      {item.status === "draft" ? (
        <textarea
          data-testid="preview.annotations-on-note"
          className="browser-note-text"
          aria-label="Your note for the agent"
          placeholder="What should change here?"
          rows={2}
          value={item.note}
          onChange={(e) => {
            onNote(e.target.value);
          }}
        />
      ) : (
        <p className="browser-note-sent">{item.note || "(no note)"}</p>
      )}
    </li>
  );
}

const NONE: readonly NoteItem[] = [];

/** The annotate controls and the collected notes of `sessionId` (only while the app preview is shown). */
export function Annotations({
  browser,
  sessionId,
  controls = true,
}: {
  browser: Browser;
  sessionId: string;
  /** Toolbar overflow owns the two entry actions; collected notes and picking feedback remain here. */
  controls?: boolean;
}): ReactElement {
  const notes = useStore(browser.store, (s) => s.notes[sessionId] ?? NONE);
  const picking = useStore(browser.store, (s) => s.picking[sessionId]);
  const drafts = notes.filter((n) => n.status === "draft").length;
  if (!controls && notes.length === 0 && picking === undefined) return <></>;
  return (
    <section className="browser-notes" aria-label="Notes on the app preview">
      <div className="browser-notes-bar">
        {picking === undefined ? (
          controls && (
            <>
              <button
                data-testid="preview.annotations-annotate"
                type="button"
                className="preview-btn"
                title="Click an element in the preview to add a note about it"
                onClick={() => void browser.annotate(sessionId, "element")}
              >
                Note on an element
              </button>
              <button
                data-testid="preview.annotations-annotate-2"
                type="button"
                className="preview-btn"
                title="Drag a box in the preview to add a note about that area"
                onClick={() => void browser.annotate(sessionId, "box")}
              >
                Note on an area
              </button>
            </>
          )
        ) : (
          <>
            <span className="browser-notes-hint" role="status">
              {picking === "box" ? "Drag a box in the preview" : "Click an element in the preview"}{" "}
              (Escape cancels)
            </span>
            <button
              data-testid="preview.annotations-cancel-pick"
              type="button"
              className="preview-btn"
              onClick={() => void browser.cancelPick(sessionId)}
            >
              Cancel
            </button>
          </>
        )}
        {drafts > 0 && (
          <button
            data-testid="preview.annotations-send"
            type="button"
            className="preview-btn browser-notes-send"
            onClick={() => {
              browser.send(sessionId);
            }}
          >
            Add {String(drafts)} {drafts === 1 ? "note" : "notes"} to the message
          </button>
        )}
        {notes.some((n) => n.status !== "draft") && (
          <button
            data-testid="preview.annotations-recheck"
            type="button"
            className="preview-btn"
            onClick={() => void browser.recheck(sessionId)}
          >
            Check again
          </button>
        )}
      </div>
      {notes.length > 0 && (
        <ol className="browser-note-list">
          {notes.map((item) => (
            <NoteRow
              key={item.id}
              item={item}
              onNote={(text) => {
                browser.setNote(sessionId, item.id, text);
              }}
              onRemove={() => {
                browser.remove(sessionId, item.id);
              }}
            />
          ))}
        </ol>
      )}
    </section>
  );
}
