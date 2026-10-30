/**
 * Caret, search and keys for the @ menu: file names from the core's search, then the other sessions of the folder
 * from the core store. Depends on core search, the core store and the mention model; called by Composer.
 */
import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import type { FileHit } from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import { NEW_CHAT } from "../../state/newChatKey";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { SEARCH_DEBOUNCE_MS } from "../search/useCoreSearch";
import {
  type MentionRow,
  chooseMention,
  chooseSession,
  mentionAt,
  mentionRows,
  sessionRows,
} from "./model/mentions";
import { keyAction, type KeyInfo } from "./model/keys";

type Answer = { scope: string; hits: readonly FileHit[]; error: string | null };

/** Shares the slash menu's key decisions; dismissed text stays in the draft. */
export function useFileMention(
  sessionId: string,
  draft: string,
  project: string | null,
  box: RefObject<HTMLTextAreaElement | null>,
) {
  const app = useApp();
  const core = app.core;
  const [caret, setCaret] = useState(0);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [selection, setSelection] = useState({ scope: "", index: 0 });
  const [answer, setAnswer] = useState<Answer | null>(null);
  const pendingCaret = useRef<number | null>(null);
  const token = mentionAt(draft, caret);
  const tokenKey = `${sessionId}:${draft}:${String(caret)}`;
  const mention = dismissed === tokenKey ? null : token;
  const query = mention?.query ?? null;
  const scope = JSON.stringify([sessionId, project, query]);
  const sessions = useCoreState((s) => s.sessions);
  const files =
    project === null || query === null || answer?.scope !== scope
      ? []
      : mentionRows(answer.hits, project, query);
  const others =
    project === null || query === null ? [] : sessionRows(sessions, project, sessionId, query);
  const rows: MentionRow[] = [
    ...files.map((hit) => ({ kind: "file" as const, hit })),
    ...others.map((session) => ({ kind: "session" as const, session })),
  ];
  const index =
    selection.scope === scope ? Math.min(selection.index, Math.max(0, rows.length - 1)) : 0;
  const error = answer?.scope === scope ? answer.error : null;
  const loading = project !== null && query !== null && answer?.scope !== scope;

  useEffect(() => {
    if (core === null || project === null || query === null) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      void core
        .run({
          type: "search",
          request: {
            op: "files",
            query,
            folder: project,
            namesOnly: true,
            limit: 8,
            ...(sessionId === NEW_CHAT ? {} : { sessionId }),
          },
        })
        .then((result) => {
          if (result.type !== "search" || result.result.kind !== "files")
            throw new Error("Unexpected file search answer");
          if (!cancelled) setAnswer({ scope, hits: result.result.hits, error: null });
        })
        .catch((thrown: unknown) => {
          if (!cancelled) setAnswer({ scope, hits: [], error: toCallError(thrown).error.message });
        });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [core, project, query, sessionId, scope]);

  useLayoutEffect(() => {
    const next = pendingCaret.current;
    if (next === null) return;
    pendingCaret.current = null;
    box.current?.focus();
    box.current?.setSelectionRange(next, next);
  }, [draft, box]);

  const pick = (row: MentionRow) => {
    if (mention === null) return;
    const next =
      row.kind === "file"
        ? chooseMention(app, sessionId, mention, row.hit)
        : chooseSession(app, sessionId, mention, row.session);
    // The next @ starts at the first row again, also for the same query.
    setSelection({ scope: "", index: 0 });
    pendingCaret.current = next;
    setCaret(next);
  };
  return {
    mention,
    rows,
    index,
    loading,
    error,
    reportCaret: () => {
      setCaret(box.current?.selectionStart ?? 0);
      setDismissed(null);
    },
    hover: (index: number) => {
      setSelection({ scope, index });
    },
    pick,
    key: (key: KeyInfo): boolean => {
      if (mention === null) return false;
      const action = keyAction(key, { running: false, slashOpen: true, hasContent: false });
      if (action === "slashClose") setDismissed(tokenKey);
      else if (action === "slashNext")
        setSelection({ scope, index: Math.max(0, Math.min(index + 1, rows.length - 1)) });
      else if (action === "slashPrev") setSelection({ scope, index: Math.max(index - 1, 0) });
      else if (action === "slashPick") {
        const row = rows[index];
        if (row !== undefined) pick(row);
      } else return false;
      return true;
    },
  };
}
