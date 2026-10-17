/**
 * The session's loops above the message box: one line while the session has any ("2 loops run by themselves", the
 * next run), and a list on request with each loop's prompt, schedule, next run and a Delete button. A loop is a
 * prompt Vibe sends as a new turn of this session on a timer, so the line is always shown while one exists: the
 * user is never surprised by turns they did not type. The list also says what the loop may do (the session's
 * access mode) and when it runs (only while Vibe Work has the session open).
 * Depends on: the composer store hooks, the composer facade (`loadLoops`, `deleteLoop`, `clearLoops`), the loops
 * model, Icon. Called by: Composer.
 * Invariants: render and event wiring only; the list is Vibe's (`loops/list`), re-read when a turn ends, after a
 * change and shortly after a loop's time has passed; nothing renders for a session without loops.
 */
import { Button } from "../../components/Controls";
import { type ReactElement, useEffect, useRef, useState } from "react";

import { Icon } from "../../components/Icon";
import { useApp } from "../shell/AppContext";
import { NONE, useComposerState, useNowSeconds } from "./hooks";
import type { AccessMode } from "./model/access";
import { everyText, loopsRun, nextText, shortPrompt, soonest } from "./model/loops";

/** How long after a loop's time a re-read waits (Vibe starts it within a second when the session is idle), seconds. */
const OVERDUE_GRACE = 2;
/** Fewest milliseconds between two re-reads of an overdue list. */
const REREAD_MS = 8_000;

/** The Loops list of `sessionId`; `mode` is the session's access mode (named in the list), `running` a turn in progress. */
export function LoopsStrip({
  sessionId,
  running,
  mode,
}: {
  sessionId: string;
  running: boolean;
  mode: AccessMode | null;
}): ReactElement | null {
  const { composer } = useApp();
  const actions = composer.actions;
  const read = useComposerState((s) => s.loops[sessionId]);
  const open = useComposerState((s) => s.loopsOpen[sessionId] === true);
  const rows = read ?? NONE;
  const active = rows.length > 0;
  const now = useNowSeconds(active);
  const [deleting, setDeleting] = useState<ReadonlySet<string>>(new Set());
  const readAt = useRef(0);
  const wasRunning = useRef(running);

  // The list is read once when the session's composer opens.
  useEffect(() => {
    void actions?.loadLoops(sessionId);
  }, [actions, sessionId]);
  // A finished turn may have added or cancelled loops (the model can schedule through its `cron` tool) and has
  // moved the next time of a loop that fired.
  useEffect(() => {
    if (wasRunning.current && !running) {
      readAt.current = Date.now();
      void actions?.loadLoops(sessionId, true);
    }
    wasRunning.current = running;
  }, [actions, sessionId, running]);
  // A loop whose time has passed has fired (or waits for an idle session): read the list again, not more than once
  // every few seconds, so its next time and a fired loop's new state show without a turn ending. The tick (`now`)
  // only re-runs this; the test uses the clock itself, so a render that still holds an old tick cannot trigger a read.
  const first = soonest(rows);
  useEffect(() => {
    if (first === null || running) return;
    const clock = Date.now();
    if (clock / 1000 < first.nextFireAt + OVERDUE_GRACE || clock - readAt.current < REREAD_MS)
      return;
    readAt.current = clock;
    void actions?.loadLoops(sessionId, true);
  }, [actions, sessionId, running, now, first]);

  if (!active || first === null) return null;
  const listId = `loops-list-${sessionId}`;
  const remove = (id: string): void => {
    setDeleting((was) => new Set(was).add(id));
    void Promise.resolve(actions?.deleteLoop(sessionId, id)).finally(() => {
      setDeleting((was) => {
        const left = new Set(was);
        left.delete(id);
        return left;
      });
    });
  };
  const toggle = open ? "Hide the loops" : "Show the loops";
  return (
    <section
      className="loops"
      aria-label="Loops"
      data-testid="composer.loops-strip"
      data-open={open}
      data-session-id={sessionId}
    >
      <div className="loops-bar">
        <Icon name="refresh-cw" size={14} />
        <span className="loops-sum" role="status">
          {loopsRun(rows.length)} · next {nextText(first, now)}
        </span>
        <button
          data-testid="composer.loops-strip-toggle"
          type="button"
          className="mini"
          aria-expanded={open}
          aria-controls={listId}
          title={toggle}
          aria-label={toggle}
          onClick={() => {
            composer.setLoopsOpen(sessionId, !open);
          }}
        >
          {open ? "Hide" : "Show"}
        </button>
      </div>
      {open ? (
        <div id={listId} className="loops-body">
          <ul className="loops-list" aria-label="Loops of this session">
            {rows.map((row) => (
              <li key={row.id} className="loops-item" data-loop-id={row.id}>
                <span className="loops-prompt" title={row.prompt}>
                  {row.prompt === "" ? "(no text)" : row.prompt}
                </span>
                <span className="loops-when">
                  {everyText(row)} · next {nextText(row, now)}
                </span>
                <Button
                  data-testid="composer.loops-strip-delete"
                  data-item-id={row.id}
                  type="button"
                  small
                  variant="danger"
                  disabled={deleting.has(row.id)}
                  title={`Delete this loop: ${shortPrompt(row.prompt, 40)}`}
                  aria-label={`Delete loop ${row.id}: ${shortPrompt(row.prompt, 40)}`}
                  onClick={() => {
                    remove(row.id);
                  }}
                >
                  Delete
                </Button>
              </li>
            ))}
          </ul>
          <p className="loops-help">
            Each loop sends its prompt as a new turn of this session when it is due and the session
            is idle.
            {mode === null
              ? " It uses the session's access mode."
              : mode.id === "auto-approve"
                ? ` ${mode.label} is on: those turns run without asking you.`
                : ` It uses the session's access mode (${mode.label}), like a message you type.`}{" "}
            Loops run while Vibe Work is open, also when you look at another session.
          </p>
          {rows.length > 1 ? (
            <Button
              data-testid="composer.loops-strip-delete-all"
              type="button"
              small
              variant="danger"
              title="Delete every loop of this session"
              aria-label="Delete every loop of this session"
              onClick={() => void actions?.clearLoops(sessionId)}
            >
              Delete all
            </Button>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
