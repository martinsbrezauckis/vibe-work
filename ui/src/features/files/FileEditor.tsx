/**
 * Quick edits in the File tab: the
 * shown text file in a plain text area (monospace, the caret at the line the tab pointed to), Save (Ctrl S / Cmd S)
 * and Cancel (Esc).
 * Saving goes through the core's `files write` (conflict check: a file that changed on disk since it was read is
 * not overwritten; the box keeps the text and offers the new version). After a save the tab reloads, the Changes
 * tab shows the edit like any other change (revert per hunk there: the normal undo path), and a context chip with
 * the diff goes into the session's composer, so the agent learns about the edit with the next message.
 * Depends on: AppContext, state/files (`write`), saveEdit. Called by: FileTab.
 * Invariants: the text never leaves the window except through `files write`; the chip is data (a resource block),
 * never typed text; Cancel and a failed save write nothing. The edit's base (file, fingerprint, original text, names)
 * is pinned when the editor opens: the tab keeps polling and may load the agent's newer version or show another file,
 * but Save always targets the pinned file with the pinned fingerprint, so a newer disk version is refused as stale
 * instead of overwritten, and a draft is never saved into a different file.
 */
import { type ReactElement, useEffect, useRef, useState } from "react";

import type { FileContent } from "../../protocol/core";
import type { Files } from "../../state/files";
import { useApp } from "../shell/AppContext";
import { saveEdit } from "./saveEdit";

/** Where line `line` (1-based) starts in `text`. */
function offsetOfLine(text: string, line: number): number {
  let at = 0;
  for (let i = 1; i < line; i++) {
    const next = text.indexOf("\n", at);
    if (next === -1) return text.length;
    at = next + 1;
  }
  return at;
}

/** The editor. `onDone` leaves edit mode (after a save or Cancel). */
export function FileEditor({
  files,
  sessionId,
  file,
  display,
  content,
  line,
  onDone,
  onDirty,
  initialText,
  onText,
}: {
  files: Files;
  sessionId: string;
  /** The tab's handle of the file being edited (pinned with the rest of the base). */
  file: string;
  /** The file's path as the tab shows it. */
  display: string;
  content: FileContent;
  line: number | null;
  onDone: () => void;
  /** Called when the draft starts or stops differing from the original (the tab guards file switching). */
  onDirty?: (dirty: boolean) => void;
  /** The draft to resume (an edit kept across a session switch); the original text when absent. */
  initialText?: string | undefined;
  /** Called with every draft change (the tab keeps it for a session switch). */
  onText?: (text: string) => void;
}): ReactElement {
  const app = useApp();
  // Pinned once: later content updates (the poll, a reload, another shown file) never change what this edit is
  // based on.
  const [base] = useState(() => ({
    file,
    fingerprint: content.fingerprint,
    original: content.text ?? "",
    name: content.name,
    display,
  }));
  const original = base.original;
  const [text, setText] = useState(initialText ?? original);
  useEffect(() => {
    onDirty?.(text !== original);
    onText?.(text);
  }, [text, original, onDirty, onText]);
  useEffect(() => () => onDirty?.(false), [onDirty]);
  // `"stale"` (the file changed on disk) or a refusal's message.
  const [problem, setProblem] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const area = useRef<HTMLTextAreaElement>(null);

  // Focus with the caret at the pointed line (the search hit's, else the start).
  useEffect(() => {
    const box = area.current;
    if (box === null) return;
    const at = offsetOfLine(original, line ?? 1);
    box.focus();
    box.setSelectionRange(at, at);
    const lineHeight = parseFloat(getComputedStyle(box).lineHeight) || 18;
    box.scrollTop = Math.max(0, ((line ?? 1) - 5) * lineHeight);
    // Only when the editor opens: later content updates must not move the caret.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const save = async () => {
    if (saving) return;
    setSaving(true);
    const result = await saveEdit(app, sessionId, original, text, "ui", base);
    setSaving(false);
    if (result.kind === "stale") setProblem("stale");
    else if (result.kind === "error") setProblem(result.message);
    else onDone();
  };

  return (
    <div className="file-edit">
      <div className="file-edit-bar" role="toolbar" aria-label="Edit the file">
        <span className="file-edit-hint">
          Editing {base.name}. <kbd>Ctrl</kbd> <kbd>S</kbd> saves, <kbd>Esc</kbd> cancels.
        </span>
        <button
          type="button"
          className="mini primary"
          disabled={saving}
          onClick={() => void save()}
        >
          {saving ? "Saving…" : "Save"}
        </button>
        <button type="button" className="mini" disabled={saving} onClick={onDone}>
          Cancel
        </button>
      </div>
      {problem !== null && (
        <p className="file-note warn" role="alert">
          {problem === "stale" ? (
            <>
              The file changed on disk since you opened it, so it was not saved. Your text is still
              here.{" "}
              <button
                type="button"
                className="mini"
                onClick={() => {
                  void files.actions.reload(sessionId).then(onDone);
                }}
              >
                Load the new version (drops your edit)
              </button>
            </>
          ) : (
            problem
          )}
        </p>
      )}
      <textarea
        ref={area}
        className="file-edit-area"
        aria-label={`Text of ${base.name}`}
        spellCheck={false}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
        }}
        onKeyDown={(e) => {
          if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "s") {
            e.preventDefault();
            void save();
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            onDone();
          } else if (e.key === "Tab" && !e.shiftKey) {
            // Tab types a tab (code); Shift Tab still leaves the box.
            e.preventDefault();
            const box = e.currentTarget;
            const { selectionStart: from, selectionEnd: to } = box;
            setText(`${text.slice(0, from)}\t${text.slice(to)}`);
            requestAnimationFrame(() => {
              box.setSelectionRange(from + 1, from + 1);
            });
          }
        }}
      />
    </div>
  );
}
