/**
 * The transcript of one session: the session's history as rows (`model/build.ts`) in a
 * virtualised list (`virtua` 0.53.3 `VList`), the pinned todo strip, "Jump to latest", the link
 * dialog and a polite live region that says when Vibe waits for an answer.
 * Depends on: model (memo, todos), rows/RowView, useFollow, TodoStrip, LinkDialog, the core and
 * transcript stores. Called by: the shell's content area (keyed by session id, so switching sessions
 * starts fresh).
 * Invariants: the component reads the store only; a session the app knows only from the listing is
 * loaded once (`ensureLoaded`); rows are rebuilt on every store change but unchanged rows keep their
 * identity, so `RowView` (memo) skips them.
 */
import { term, TEXT } from "../../vocabulary";
import { type ReactElement, useEffect, useMemo, useRef } from "react";
import { useStore } from "../../state/createStore";
import { useCoveredByPage } from "../shell/coverContext";
import { Icon } from "../../components/Icon";
import { VList, type VListHandle } from "virtua";

import type { JsonValue } from "../../protocol/core";
import { FileRefsProvider } from "../files/FileRef";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { useTranscriptUi } from "./hooks";
import { FindBar } from "./FindBar";
import { LinkDialog } from "./LinkDialog";
import { SourcesPanel } from "./SourcesPanel";
import type { Row } from "./model/build";
import { createTranscriptBuilder } from "./model/memo";
import { subagentRows } from "./model/subagentRows";
import { latestTodos } from "./model/todos";
import { RowView } from "./rows/RowView";
import { TodoStrip } from "./TodoStrip";
import { useFollow } from "./useFollow";

const NONE: readonly JsonValue[] = [];

function list(value: JsonValue | undefined): readonly JsonValue[] {
  return Array.isArray(value) ? value : NONE;
}

/** What the live region says: an open callback first, else the running group's status. */
function liveText(rows: readonly Row[]): string {
  if (rows.some((row) => row.kind === "callback")) return "Vibe is waiting for your answer.";
  const last = rows[rows.length - 1];
  return last?.kind === "steps" && last.live ? last.statusText || "Working" : "";
}

/** Renders the transcript of `sessionId`. */
export function Transcript({ sessionId }: { sessionId: string }): ReactElement {
  const covered = useCoveredByPage();
  const { transcript, find } = useApp();
  const finding = useStore(find.store, (s) => s.conversation?.sessionId === sessionId);
  const entry = useCoreState((s) => s.sessions[sessionId]);
  const loading = useTranscriptUi((s) => s.loading[sessionId] === true);
  const builder = useMemo(() => createTranscriptBuilder(), []);
  const history = list(entry?.state?.["history"]);
  const built = builder.build(history, list(entry?.state?.["turns"]));
  // a subagent's open card (its child session is not loaded) is shown at the end of the parent's
  // transcript, under "Waiting for a subagent", and answered for the child.
  const cards = useCoreState((s) => s.subagentCards);
  const childSessions = entry?.state?.["childSessions"];
  const childRows = useMemo(
    () => subagentRows(cards, sessionId, childSessions),
    [cards, sessionId, childSessions],
  );
  const rows = useMemo(
    () => (childRows.length === 0 ? built : [...built, ...childRows]),
    [built, childRows],
  );
  const todos = useMemo(() => latestTodos(history), [history]);
  const listRef = useRef<VListHandle>(null);
  const attention = rows.reduce((last, row, index) => (row.kind === "callback" ? index : last), -1);
  const follow = useFollow(listRef, rows.length, rows, { paused: finding, attention });
  const unloaded = entry !== undefined && entry.state === null && !entry.localOnly;

  useEffect(() => {
    if (unloaded) void transcript.ensureLoaded(sessionId);
  }, [transcript, sessionId, unloaded]);

  if (entry === undefined || unloaded || loading) {
    return (
      <div className="tx tx-empty" role="status">
        {entry === undefined ? `${TEXT.thisSession} is not loaded.` : "Loading the session…"}
      </div>
    );
  }
  return (
    <FileRefsProvider sessionId={sessionId}>
      <div className="tx" {...follow.intent}>
        {/* find in this conversation (Ctrl F, or a message result of the global search). */}
        <FindBar sessionId={sessionId} rows={rows} listRef={listRef} />
        {todos !== null && todos.length > 0 ? <TodoStrip todos={todos} /> : null}
        {rows.length === 0 ? (
          <div className="tx-empty">
            <p>No messages yet. Vibe&apos;s answers, steps and approvals appear here.</p>
          </div>
        ) : (
          <VList
            ref={listRef}
            data={rows}
            className="tx-list"
            aria-label={term("workNames", "session").name}
            role="log"
            tabIndex={0}
            onScroll={follow.onScroll}
            onResize={follow.onResize}
          >
            {(row: Row) => <RowView key={row.key} row={row} sessionId={sessionId} />}
          </VList>
        )}
        {follow.pinned ? null : (
          <button
            data-testid="transcript.transcript-jump-latest"
            type="button"
            className="jump-latest"
            title="Jump to latest message"
            aria-label="Jump to latest message"
            onClick={follow.jump}
          >
            <Icon name="arrow-down" size={14} />
            Jump to latest
          </button>
        )}
        <p className="sr" aria-live="polite">
          {liveText(rows)}
        </p>
        {/* Not while a page covers the work view: the page shows its own content. */}
        {!covered && <SourcesPanel />}
        {!covered && <LinkDialog />}
      </div>
    </FileRefsProvider>
  );
}
