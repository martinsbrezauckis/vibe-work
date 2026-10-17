/**
 * Find in this file (Ctrl F in the File tab covers that file and shows "In: <name>").
 * The lines that hold every word, "n of m", ↑ ↓; the current line is the line the text view scrolls to and marks.
 * Depends on: AppContext, state/findStore, search/findText, search/searchRoutes (`publishCount`). Called by: FileTab.
 * Invariants: searches only the text the tab shows (the first 256 KB of a big file, which the tab says); never
 * changes the text view's DOM (CSS Custom Highlight API marks).
 */
import { type ReactElement, useEffect, useMemo, useRef } from "react";

import { Icon } from "../../components/Icon";
import { useStore } from "../../state/createStore";
import { findWords, holdsAll, markWords } from "../search/findText";
import { publishCount } from "../search/searchRoutes";
import { useApp } from "../shell/AppContext";

/** The line numbers (1-based) of `text` that hold every word of `query`. */
export function fileMatches(text: string, query: string): number[] {
  const words = findWords(query);
  if (words.length === 0) return [];
  const out: number[] = [];
  text.split("\n").forEach((line, i) => {
    if (holdsAll(line, words)) out.push(i + 1);
  });
  return out;
}

/** The current match's line (for the text view), or `null` when the bar is closed or finds nothing. */
export function useFileFindLine(sessionId: string, text: string | null): number | null {
  const app = useApp();
  const bar = useStore(app.find.store, (s) => s.file);
  const open = bar !== null && bar.sessionId === sessionId;
  const query = open ? bar.query : "";
  const matches = useMemo(
    () => (open && text !== null ? fileMatches(text, query) : []),
    [open, text, query],
  );
  if (!open || matches.length === 0) return null;
  return matches[Math.min(bar.at, matches.length - 1)] ?? null;
}

/** The bar, or nothing while this session's file bar is closed. */
export function FileFindBar({
  sessionId,
  name,
  text,
}: {
  sessionId: string;
  name: string;
  text: string;
}): ReactElement | null {
  const app = useApp();
  const bar = useStore(app.find.store, (s) => s.file);
  const open = bar !== null && bar.sessionId === sessionId;
  const query = open ? bar.query : "";
  const matches = useMemo(() => fileMatches(text, query), [text, query]);
  const input = useRef<HTMLInputElement>(null);
  const at = matches.length === 0 ? -1 : Math.min(bar?.at ?? 0, matches.length - 1);
  const current = at >= 0 ? (matches[at] ?? null) : null;

  useEffect(() => {
    if (open) publishCount(app, "file", matches.length);
  }, [app, open, matches.length]);
  useEffect(() => {
    if (open) input.current?.focus();
  }, [open, bar?.opened]);
  // Mark the words in the matching lines once the text view scrolled to the current one.
  useEffect(() => {
    if (!open || matches.length === 0) return;
    const root = document.querySelector(".file-text");
    if (root === null) return;
    const lines = matches
      .map((n) => root.querySelector(`[data-line="${String(n)}"]`))
      .filter((el): el is Element => el !== null);
    const here = current === null ? null : root.querySelector(`[data-line="${String(current)}"]`);
    return markWords(lines, here, findWords(query));
  }, [open, matches, current, query]);

  if (!open) return null;
  const step = (by: 1 | -1) => {
    app.find.actions.step("file", by, matches.length);
  };
  const close = () => {
    app.find.actions.close("file");
  };
  return (
    <div className="find-bar file-find" role="search" aria-label={`Find in ${name}`}>
      <span className="find-scope">In: {name}</span>
      <input
        data-testid="files.file-find-bar-set-query"
        ref={input}
        type="search"
        aria-label={`Find in ${name}`}
        placeholder="Find in this file"
        value={query}
        onChange={(e) => {
          app.find.actions.setQuery("file", e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            step(e.key === "ArrowUp" || (e.key === "Enter" && e.shiftKey) ? -1 : 1);
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            close();
          }
        }}
      />
      <span className="find-count" role="status" aria-live="polite">
        {query.trim() === ""
          ? ""
          : matches.length === 0
            ? "No matches"
            : `${String(at + 1)} of ${String(matches.length)}`}
      </span>
      <button
        data-testid="files.file-find-bar-step"
        type="button"
        aria-label="Previous match"
        disabled={matches.length === 0}
        onClick={() => {
          step(-1);
        }}
      >
        <Icon name="arrow-up" size={14} />
      </button>
      <button
        data-testid="files.file-find-bar-step-2"
        type="button"
        aria-label="Next match"
        disabled={matches.length === 0}
        onClick={() => {
          step(1);
        }}
      >
        <Icon name="arrow-down" size={14} />
      </button>
      <button
        data-testid="files.file-find-bar-button"
        type="button"
        aria-label="Close find"
        onClick={close}
      >
        <Icon name="x" size={14} />
      </button>
    </div>
  );
}
