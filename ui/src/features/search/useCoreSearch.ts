/**
 * The palette's messages and files results: the core's `search` command, asked a moment after the
 * typing stops, and again while the message index is still filling ("Searching older conversations…").
 * Depends on: react, AppContext, protocol types. Called by: CommandPalette.
 * Invariants: each kind publishes independently; cancelled answers never update state or schedule another poll.
 * Pending rows must still match the new words; usePaletteResults owns stable display ordering.
 */
import { useEffect, useMemo, useState } from "react";

import type { FileHit, IndexStatus, MessageHit, SearchResult } from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import { useApp } from "../shell/AppContext";
import { findWords, holdsAll } from "./findText";

/** Wait after the last key before asking the core. */
export const SEARCH_DEBOUNCE_MS = 150;
/** How often the search runs again while the index fills. */
export const FILLING_POLL_MS = 700;
/** Hits asked per kind (the palette shows five per group, all with "Show all"). */
const LIMIT = 50;

/** What the palette shows from the core. */
export interface CoreResults {
  /** The query these results are for. */
  query: string;
  messages: readonly MessageHit[];
  files: readonly FileHit[];
  messageTotal: number;
  fileTotal: number;
  index: IndexStatus | null;
  filesTruncated: boolean;
  loading: boolean;
  error: string | null;
}

const EMPTY: CoreResults = {
  query: "",
  messages: [],
  files: [],
  messageTotal: 0,
  fileTotal: 0,
  index: null,
  filesTruncated: false,
  loading: false,
  error: null,
};

/** Searches messages and files for `query` (trimmed); returns the latest answer. */
export function useCoreSearch(query: string): CoreResults {
  const { core } = useApp();
  const [results, setResults] = useState<CoreResults>(EMPTY);
  const wanted = query.trim();

  useEffect(() => {
    if (core === null || wanted === "") return;
    let cancelled = false;
    let messagesPending = true;
    let filesPending = true;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const ask = async (op: "messages" | "files"): Promise<void> => {
      try {
        const answer = await core.run({
          type: "search",
          request: { op, query: wanted, limit: LIMIT },
        });
        if (
          answer.type !== "search" ||
          answer.result.kind === "status" ||
          answer.result.kind !== op
        )
          throw new Error("Unexpected search answer");
        const result = answer.result;
        if (cancelled) return;
        if (op === "messages") messagesPending = false;
        else filesPending = false;
        setResults((previous) =>
          updateResults(forQuery(previous, wanted), result, messagesPending || filesPending),
        );
        if (result.kind === "messages" && result.index.filling)
          timer = setTimeout(() => {
            messagesPending = true;
            void ask("messages");
          }, FILLING_POLL_MS);
      } catch (thrown) {
        if (cancelled) return;
        if (op === "messages") messagesPending = false;
        else filesPending = false;
        setResults((previous) => ({
          ...forQuery(previous, wanted),
          loading: messagesPending || filesPending,
          error: toCallError(thrown).error.message,
        }));
      }
    };
    timer = setTimeout(() => {
      void ask("messages");
      void ask("files");
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      if (timer !== null) clearTimeout(timer);
    };
  }, [core, wanted]);

  return useMemo(
    () =>
      wanted === ""
        ? EMPTY
        : core === null
          ? { ...EMPTY, query: wanted, error: "The core search is unavailable in this window." }
          : forQuery(results, wanted),
    [core, wanted, results],
  );
}

function forQuery(results: CoreResults, query: string): CoreResults {
  if (results.query === query) return results;
  const words = findWords(query);
  const messages = results.messages.filter((hit) => holdsAll(hit.snippet, words));
  const files = results.files.filter((hit) => holdsAll(`${hit.path}\n${hit.text}`, words));
  return {
    ...EMPTY,
    query,
    messages,
    files,
    messageTotal: messages.length,
    fileTotal: files.length,
    loading: true,
  };
}

function updateResults(
  previous: CoreResults,
  result: Extract<SearchResult, { kind: "messages" | "files" }>,
  loading: boolean,
): CoreResults {
  if (result.kind === "messages")
    return {
      ...previous,
      loading,
      messages: result.hits,
      messageTotal: result.total,
      index: result.index,
    };
  return {
    ...previous,
    loading,
    files: result.hits,
    fileTotal: result.total,
    filesTruncated: result.truncated,
  };
}
