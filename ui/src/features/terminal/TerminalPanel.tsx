/**
 * The side panel's Terminal tab: the shown session's terminal tabs, a toolbar (new,
 * restart, clear, copy, paste, close) and the active terminal's view. Switching sessions switches
 * terminals; a hidden session's terminals keep running in the core and buffer their output.
 * Depends on: store.ts, model.ts, TerminalView (lazy), features/shell (app context, shell state),
 * components/Icon, pages/point ("Point Vibe at this": the selection as a context chip). Called by: register.tsx (the panel tab body).
 * Invariants: renders only; every action is a store method; no terminal is opened for the
 * new-session view (there is no session folder yet).
 */
import { Suspense, lazy, useEffect, useRef, useState } from "react";

import { Icon } from "../../components/Icon";
import { useStore } from "../../state/createStore";
import { detectMac } from "../../state/shortcuts";
import { pointTerminal } from "../pages/point";
import { useApp } from "../shell/AppContext";
import { useShellState } from "../shell/hooks";
import { activeOf } from "./model";
import type { MountedTerminal } from "./session";
import type { Terminals } from "./store";
import { ViewBoundary } from "./ViewBoundary";

const TerminalView = lazy(() => import("./TerminalView"));
/** Copy/paste keys differ on macOS (session.ts uses the same rule). */
const MAC = detectMac();

/** A toolbar button. */
function Tool({
  testId,
  label,
  onClick,
  children,
}: {
  testId: string;
  label: string;
  onClick: () => void;
  children: string;
}) {
  return (
    <button
      data-testid={testId}
      type="button"
      className="vw-term-tool"
      title={label}
      aria-label={label}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/** The Terminal tab body. */
export function TerminalPanel({ terminals }: { terminals: Terminals }) {
  const app = useApp();
  const sessionId = useShellState((s) => s.selected);
  const [noticeText, setNoticeText] = useState<{ sessionId: string; text: string } | null>(null);
  const list = useStore(terminals.store, (s) =>
    sessionId === null ? undefined : s.bySession[sessionId],
  );
  const active = useStore(terminals.store, (s) =>
    sessionId === null ? null : activeOf(s, sessionId),
  );
  const error = useStore(terminals.store, (s) =>
    sessionId === null ? undefined : s.error[sessionId],
  );
  const mounted = useRef<MountedTerminal | null>(null);
  const tabsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (sessionId !== null) void terminals.ensure(sessionId);
  }, [terminals, sessionId]);

  if (sessionId === null) {
    return (
      <div className="empty">
        Open a session to use its terminal (it starts in the session folder).
      </div>
    );
  }
  const tabs = list ?? [];
  const notice = (text: string) => {
    setNoticeText({ sessionId, text });
  };
  const leave = () => {
    tabsRef.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.focus();
  };
  const exited = active?.state.type === "exited" ? active.state : null;

  return (
    <div className="vw-term-panel" data-session-id={sessionId}>
      <div className="vw-term-bar">
        <div
          data-testid="terminal.terminal-panel-vw-term-tabs"
          className="vw-term-tabs"
          role="tablist"
          aria-label="Terminals"
          ref={tabsRef}
        >
          {tabs.map((t, index) => (
            <button
              data-testid="terminal.terminal-panel-select"
              data-item-id={t.terminalId}
              key={t.terminalId}
              type="button"
              role="tab"
              className="vw-term-tab"
              aria-selected={t.terminalId === active?.terminalId}
              data-terminal-id={t.terminalId}
              data-state={t.state.type}
              onClick={() => {
                terminals.select(sessionId, t.terminalId);
              }}
            >
              {`${String(index + 1)} ${t.shell.split(/[\\/]/).pop() ?? "shell"}`}
            </button>
          ))}
          <button
            data-testid="terminal.terminal-panel-open"
            type="button"
            className="ibtn vw-term-new"
            aria-label="New terminal"
            title="New terminal"
            onClick={() => void terminals.open(sessionId)}
          >
            <Icon name="plus" size={14} />
          </button>
        </div>
        {active !== null && (
          <div className="vw-term-tools">
            <Tool
              testId="terminal.toolbar-copy"
              label={`Copy the selection (${MAC ? "Cmd+C" : "Ctrl+Shift+C"})`}
              onClick={() => void mounted.current?.copy()}
            >
              Copy
            </Tool>
            <Tool
              testId="terminal.toolbar-paste"
              label={`Paste (${MAC ? "Cmd+V" : "Ctrl+Shift+V"})`}
              onClick={() => void mounted.current?.paste()}
            >
              Paste
            </Tool>
            <Tool
              testId="terminal.toolbar-clear"
              label="Clear the view"
              onClick={() => mounted.current?.clear()}
            >
              Clear
            </Tool>
            <Tool
              testId="terminal.toolbar-action-1"
              label="Point Vibe at this: add the selected text to your next message"
              onClick={() => {
                const outcome = pointTerminal(app, sessionId);
                notice(
                  outcome.ok ? `Added to your message: ${outcome.chip.label}` : outcome.reason,
                );
              }}
            >
              Point Vibe at this
            </Tool>
            <Tool
              testId="terminal.toolbar-restart"
              label="Restart the shell (stops its processes)"
              onClick={() => void terminals.restart(sessionId, active.terminalId)}
            >
              Restart
            </Tool>
            <Tool
              testId="terminal.toolbar-close"
              label="Close the terminal (stops its processes)"
              onClick={() => void terminals.close(sessionId, active.terminalId)}
            >
              Close
            </Tool>
          </div>
        )}
      </div>
      {error !== undefined && (
        <div className="key-problem" role="alert">
          {error}
        </div>
      )}
      {noticeText?.sessionId === sessionId && (
        <div className="vw-term-exited" role="status">
          {noticeText.text}
        </div>
      )}
      {exited !== null && active !== null && (
        <div className="vw-term-exited" role="status">
          {`The shell exited${exited.exitCode === null ? "" : ` with code ${String(exited.exitCode)}`}${exited.signal === null ? "" : ` (${exited.signal})`}.`}
          <button
            data-testid="terminal.terminal-panel-restart"
            type="button"
            className="vw-term-tool"
            onClick={() => void terminals.restart(sessionId, active.terminalId)}
          >
            Restart
          </button>
        </div>
      )}
      <div className="vw-term-body">
        {active === null ? (
          <div className="empty">
            {list === undefined ? "Loading terminals..." : "No terminal open."}
          </div>
        ) : (
          <ViewBoundary key={active.terminalId}>
            <Suspense
              fallback={
                <div className="empty" aria-busy="true">
                  Loading the terminal...
                </div>
              }
            >
              <TerminalView
                key={active.terminalId}
                terminals={terminals}
                sessionId={sessionId}
                terminalId={active.terminalId}
                onMounted={(m) => {
                  mounted.current = m;
                }}
                notice={notice}
                leave={leave}
              />
            </Suspense>
          </ViewBoundary>
        )}
      </div>
    </div>
  );
}
