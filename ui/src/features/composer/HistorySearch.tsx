/**
 * Prompt search (Ctrl+R, also on macOS): a search field over the earlier prompts above the composer, newest first.
 * Typing filters by words; Down or Ctrl+R again moves to an older match, Up to a newer one; Enter or Tab puts the
 * selected prompt into the draft (never sends); Esc, or focus leaving the search, closes it and keeps the earlier draft.
 * Depends on: the prompt history list in the composer store (already filtered by the core). Called by: Composer.
 * Invariants: the list is the window's copy of the shared history; nothing here reaches the core or the model. The
 * count line is no live region: it is said once by the app's polite announcer when the search opens and once typing
 * pauses (a status region inserted with its text is not read, and the selected prompt's own announcement on the same
 * keystroke would take its place).
 */
import { type ReactElement, useEffect, useMemo, useRef, useState } from "react";

import { announce } from "../../state/announcer";
import { searchHistory } from "../../state/promptHistory";

/** How long typing must pause before the count line is said. VoiceOver does not re-read this combobox field after
 * typing (it does the Settings search field, which therefore waits longer), so the line follows the selected prompt. */
const SAY_COUNT_MS = 600;

/** True for Ctrl+R alone (Control+R on macOS): the key that opens the search and moves to an older match. */
export function isHistorySearchKey(key: {
  key: string;
  code?: string;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}): boolean {
  return (
    key.ctrlKey &&
    !key.altKey &&
    !key.metaKey &&
    !key.shiftKey &&
    (key.code === "KeyR" || key.key.toLowerCase() === "r")
  );
}

/** A long prompt shows its first line, followed by "…" when more follows. */
function firstLine(text: string): string {
  const line = text.split("\n", 1)[0] ?? "";
  return line.length < text.length ? `${line}…` : line;
}

/** The prompt search field and its list of matching earlier prompts, with the selected one highlighted. */
export function HistorySearch({
  menuId,
  entries,
  onLoad,
  onCancel,
}: {
  menuId: string;
  entries: readonly string[];
  /** Puts the picked prompt into the draft and closes the search. */
  onLoad: (text: string) => void;
  /** Closes the search, keeping the draft as it was before it opened; `refocus` returns focus to the composer. */
  onCancel: (refocus: boolean) => void;
}): ReactElement {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const field = useRef<HTMLInputElement>(null);
  // Set before handing focus back, so the field's own blur does not close the search a second time.
  const done = useRef(false);
  const load = (text: string) => {
    done.current = true;
    onLoad(text);
  };
  const cancel = (refocus: boolean) => {
    done.current = true;
    onCancel(refocus);
  };
  const selected = useRef<HTMLDivElement | null>(null);
  const matches = useMemo(() => searchHistory(entries, query), [entries, query]);
  const active = Math.min(index, Math.max(matches.length - 1, 0));
  const current = matches[active];
  useEffect(() => {
    field.current?.focus();
  }, []);
  useEffect(() => {
    // jsdom has no scrollIntoView; a real webview keeps the selected prompt in sight.
    const row = selected.current;
    const scroll: unknown = row === null ? undefined : Reflect.get(row, "scrollIntoView");
    if (row !== null && typeof scroll === "function") scroll.call(row, { block: "nearest" });
  }, [active, matches]);
  const listId = `${menuId}-history`;
  const optionId = (i: number) => `${listId}-option-${String(i)}`;
  const status =
    entries.length === 0
      ? "Prompts you send will appear here."
      : matches.length === 0
        ? "No earlier prompt matches."
        : matches.length === 1
          ? "1 prompt found"
          : `${String(matches.length)} prompts found`;
  useEffect(() => {
    const timer = setTimeout(() => {
      announce(status);
    }, SAY_COUNT_MS);
    return () => {
      clearTimeout(timer);
    };
  }, [status, query]);
  return (
    <div
      data-testid="composer.history-search"
      className="slash history-search"
      onBlur={(e) => {
        if (!done.current && !e.currentTarget.contains(e.relatedTarget)) cancel(false);
      }}
    >
      <input
        ref={field}
        data-testid="composer.history-search-field"
        data-keeps-ctrl-r=""
        className="history-search-field"
        type="search"
        role="combobox"
        aria-label="Search earlier prompts"
        placeholder="Search earlier prompts"
        aria-expanded={matches.length > 0}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={current === undefined ? undefined : optionId(active)}
        autoComplete="off"
        spellCheck={false}
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setIndex(0);
        }}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing) return;
          const older = e.key === "ArrowDown" || isHistorySearchKey(e);
          if (older || e.key === "ArrowUp") {
            e.preventDefault();
            e.stopPropagation();
            setIndex(
              older
                ? Math.min(active + 1, Math.max(matches.length - 1, 0))
                : Math.max(active - 1, 0),
            );
          } else if (e.key === "Enter" || (e.key === "Tab" && !e.shiftKey)) {
            e.preventDefault();
            e.stopPropagation();
            if (current !== undefined) load(current.text);
            else if (e.key === "Enter") cancel(true);
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            cancel(true);
          }
        }}
      />
      <div className="slash-empty">{status}</div>
      <div
        data-testid="composer.history-search-list"
        className="slash-list"
        role="listbox"
        id={listId}
        aria-label="Earlier prompts"
      >
        {matches.map((match, i) => (
          <div
            key={match.index}
            id={optionId(i)}
            data-testid="composer.history-search-item"
            data-item-id={`history:${String(match.index)}`}
            className="it"
            role="option"
            aria-selected={i === active}
            title={match.text}
            ref={i === active ? selected : undefined}
            onMouseDown={(e) => {
              e.preventDefault();
            }}
            onClick={() => {
              load(match.text);
            }}
            onMouseMove={() => {
              if (i !== active) setIndex(i);
            }}
          >
            <span>{firstLine(match.text)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
