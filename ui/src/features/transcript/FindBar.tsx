/**
 * Find in this conversation (Ctrl F in a conversation covers that conversation,
 * the field says "In: This conversation", "2 of 5" with ↑ ↓, Esc closes and keeps your place). A message result of
 * the global search opens this bar at that message.
 * Depends on: AppContext, state/findStore, model/find, search/findText, search/searchRoutes (`publishCount`), virtua
 * (`VListHandle`). Called by: Transcript.
 * Invariants: the transcript's DOM is never changed (words are marked with the CSS Custom Highlight API; without it
 * the current message gets the `find-current` outline only); a message the result names but the transcript has not
 * loaded is read once with Vibe's largest page (500 entries) before the bar says it is further back.
 */
import { type ReactElement, type RefObject, useEffect, useMemo, useRef, useState } from "react";
import type { VListHandle } from "virtua";

import { Icon } from "../../components/Icon";
import { formatKeys } from "../../state/shortcuts";
import { useStore } from "../../state/createStore";
import { findWords, markWords } from "../search/findText";
import { publishCount } from "../search/searchRoutes";
import { useApp } from "../shell/AppContext";
import type { Row } from "./model/build";
import { findMatches } from "./model/find";

/** Vibe's largest history page (`PageRequest.limit`, `protocol.py:287-290`). */
const DEEPEST_READ = 500;

/** The bar, or nothing when this session's bar is closed. */
export function FindBar({
  sessionId,
  rows,
  listRef,
}: {
  sessionId: string;
  rows: readonly Row[];
  listRef: RefObject<VListHandle | null>;
}): ReactElement | null {
  const app = useApp();
  const bar = useStore(app.find.store, (s) => s.conversation);
  const open = bar !== null && bar.sessionId === sessionId;
  const query = open ? bar.query : "";
  const matches = useMemo(() => findMatches(rows, query), [rows, query]);
  const input = useRef<HTMLInputElement>(null);
  const readDeeper = useRef(false);
  const [deeperDone, setDeeperDone] = useState(false);
  const at = matches.length === 0 ? -1 : Math.min(bar?.at ?? 0, matches.length - 1);
  const current = at >= 0 ? matches[at] : undefined;
  const findLabel = `Find in this conversation (${formatKeys("Mod+F")})`;

  useEffect(() => {
    if (open) publishCount(app, "conversation", matches.length);
  }, [app, open, matches.length]);

  // Focus the field whenever the bar is (re)opened.
  useEffect(() => {
    if (open) input.current?.focus();
  }, [open, bar?.opened]);

  // A message result names its entry: start there (reading older history once when it is not loaded yet).
  const target = open ? bar.target : null;
  useEffect(() => {
    if (target === null) return;
    const index = matches.findIndex((m) => m.entryId === target);
    if (index >= 0) {
      app.find.actions.jump("conversation", index);
      return;
    }
    const loaded = rows.some((r) => "entry" in r && r.entry["id"] === target);
    if (loaded) {
      // Loaded but not a match for these words (the words changed): start at the first match.
      app.find.actions.jump("conversation", 0);
      return;
    }
    if (!readDeeper.current) {
      readDeeper.current = true;
      const page = { limit: DEEPEST_READ };
      void app.core
        ?.run({
          type: "call",
          method: "session/read",
          params: { sessionId, history: page, turns: page },
        })
        .catch(() => undefined)
        .finally(() => {
          setDeeperDone(true);
        });
      return;
    }
    if (!deeperDone) return;
    app.find.actions.jump("conversation", 0, true);
  }, [app, target, matches, rows, sessionId, deeperDone]);

  // Scroll the current match into the middle and mark the words.
  useEffect(() => {
    if (!open || current === undefined) return;
    listRef.current?.scrollToIndex(current.index, { align: "center" });
    let cleanup: () => void = () => undefined;
    const timer = setTimeout(() => {
      const root = document.querySelector(".tx-list");
      if (root === null) return;
      const ids = new Set(matches.map((m) => m.entryId));
      const shown = [...root.querySelectorAll("[data-entry-id]")].filter((el) =>
        ids.has(el.getAttribute("data-entry-id") ?? ""),
      );
      const here = shown.find((el) => el.getAttribute("data-entry-id") === current.entryId) ?? null;
      here?.classList.add("find-current");
      const unmark = markWords(shown, here, findWords(query));
      cleanup = () => {
        unmark();
        here?.classList.remove("find-current");
      };
    }, 60);
    return () => {
      clearTimeout(timer);
      cleanup();
    };
  }, [open, current, matches, query, listRef]);

  if (!open) return null;
  const close = () => {
    app.find.actions.close("conversation");
  };
  const step = (by: 1 | -1) => {
    app.find.actions.step("conversation", by, matches.length);
  };
  return (
    <div className="find-bar" role="search" aria-label="Find in this conversation">
      <button
        type="button"
        className="find-scope"
        title="Search all with these words"
        aria-label="Search all with these words"
        onClick={() => {
          close();
          app.shell.actions.setOverlay("palette", query);
        }}
      >
        In: This conversation <Icon name="x" size={12} />
      </button>
      <input
        ref={input}
        type="search"
        aria-label={findLabel}
        title={findLabel}
        placeholder="Find in messages"
        value={query}
        onChange={(e) => {
          app.find.actions.setQuery("conversation", e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            step(e.key === "ArrowUp" || (e.key === "Enter" && e.shiftKey) ? -1 : 1);
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            close();
          } else if (e.key === "Backspace" && query === "") {
            close();
            app.shell.actions.setOverlay("palette", "");
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
        type="button"
        aria-label="Previous match (Shift+Enter or Up Arrow)"
        title="Previous match (Shift+Enter or Up Arrow)"
        onClick={() => {
          step(-1);
        }}
        disabled={matches.length === 0}
      >
        <Icon name="arrow-up" size={14} />
      </button>
      <button
        type="button"
        aria-label="Next match (Enter or Down Arrow)"
        title="Next match (Enter or Down Arrow)"
        onClick={() => {
          step(1);
        }}
        disabled={matches.length === 0}
      >
        <Icon name="arrow-down" size={14} />
      </button>
      <button
        type="button"
        aria-label="Close find (Escape)"
        title="Close find (Escape)"
        onClick={close}
      >
        <Icon name="x" size={14} />
      </button>
      {bar.older && (
        <p className="find-note">
          That message is further back than this conversation shows (the newest {DEEPEST_READ}{" "}
          entries).
        </p>
      )}
    </div>
  );
}
