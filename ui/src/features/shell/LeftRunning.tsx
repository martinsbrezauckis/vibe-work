/**
 * The Processes panel combines the open session and "Left running" lists: the status bar button that
 * appears while the open session has dev servers or long commands running, or processes from deleted sessions
 * still run, and the dialog with both lists: "This session" (live output and Stop, `SessionProcessesSection`) and
 * "Left running" (the agent's dev servers whose session was deleted, with Stop). Why the status bar and not a
 * Settings page: a server holding a port is live state the user should notice, not a preference; the button costs
 * nothing while both lists are empty.
 * Depends on: AppContext, hooks/useModalFocus, state/createStore, state/leftRunning, state/sessionProcesses,
 * SessionProcessesSection. Called by: StatusBar.
 * Invariants: the dialog is modal (`aria-modal`), so the native child views hide while it shows
 * (`NativeViewport`); Close has the initial focus; Escape closes it. Stop never asks again: the click is the
 * decision (the window's own trusted surface).
 */
import { type ReactElement, useRef } from "react";

import { Button as ActionButton } from "../../components/Controls";
import { useModalFocus } from "../../hooks/useModalFocus";
import { processLabel } from "../../state/leftRunning";
import { createStore, useStore } from "../../state/createStore";
import type { SessionProcessesState } from "../../state/sessionProcesses";
import { useApp } from "./AppContext";
import { SessionProcessesSection } from "./SessionProcessesSection";

/** Stands in for the session processes' store when the app has no core. */
const NO_PROCS = createStore<SessionProcessesState>({
  sessionId: null,
  items: [],
  outputs: {},
  stopping: [],
});

/** The status bar button; nothing while the list is empty. */
export function LeftRunningButton(): ReactElement | null {
  const { leftRunning } = useApp();
  if (leftRunning === null) return null;
  return <Button data-testid="shell.left-running-button" left={leftRunning} />;
}

function Button({ left }: { left: NonNullable<ReturnType<typeof useApp>["leftRunning"]> }) {
  const { sessionProcesses } = useApp();
  const count = useStore(left.store, (s) => s.items.length);
  const running = useStore(
    sessionProcesses?.store ?? NO_PROCS,
    (s) => s.items.filter((i) => i.status === "running").length,
  );
  const parts = [
    ...(running > 0 ? [`${String(running)} running`] : []),
    ...(count > 0 ? [`${String(count)} left running`] : []),
  ];
  return (
    <>
      {parts.length > 0 && (
        <button
          type="button"
          className="sb-usage sb-left"
          data-testid="shell.statusbar-left-running"
          title="Dev servers and long commands: this session's, and those deleted sessions left running. Click to see their output and stop them."
          aria-haspopup="dialog"
          onClick={() => {
            left.open();
          }}
        >
          {parts.join(" · ")}
        </button>
      )}
      {/* Rendered apart from the button: stopping the last process keeps the dialog open ("Nothing is left"). */}
      <LeftRunningDialog left={left} />
    </>
  );
}

/** The list dialog, while open. */
function LeftRunningDialog({
  left,
}: {
  left: NonNullable<ReturnType<typeof useApp>["leftRunning"]>;
}): ReactElement | null {
  const state = useStore(left.store, (s) => s);
  const { sessionProcesses } = useApp();
  const dialog = useRef<HTMLDivElement>(null);
  const close = () => {
    left.close();
  };
  useModalFocus(state.open, dialog, close);
  if (!state.open) return null;
  return (
    <>
      <div data-testid="shell.left-running-scrim" className="scrim" onClick={close} />
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="left-running-title"
        data-testid="shell.left-running-dialog"
        ref={dialog}
        tabIndex={-1}
      >
        <header>
          <h2 id="left-running-title">Processes</h2>
        </header>
        {sessionProcesses !== null && <SessionProcessesSection procs={sessionProcesses} />}
        <section className="proc-section">
          <h3>Left running</h3>
          <p className="left-running-intro">
            The agent started these and they kept running after their session was deleted. Stop ends
            a process and the processes it started.
          </p>
          {state.items.length === 0 ? (
            <p data-testid="left-running-empty">Nothing is left running.</p>
          ) : (
            <ul className="left-running-list">
              {state.items.map((item) => (
                <li data-item-id={item.id} key={item.id} data-left-running={item.id}>
                  <span className="left-running-what">
                    <span>{processLabel(item)}</span>
                    <span className="d">
                      {item.sessionTitle === null
                        ? "from a deleted session"
                        : `from “${item.sessionTitle}” (deleted)`}
                      {` · pid ${String(item.pid)}`}
                    </span>
                  </span>
                  <ActionButton
                    data-testid="shell.left-running-stop"
                    variant="danger"
                    shape="outlined"
                    disabled={state.stopping.includes(item.id)}
                    aria-label={`Stop ${processLabel(item)}`}
                    onClick={() => {
                      void left.stop(item.id);
                    }}
                  >
                    {state.stopping.includes(item.id) ? "Stopping…" : "Stop"}
                  </ActionButton>
                </li>
              ))}
            </ul>
          )}
        </section>
        <footer className="modal-actions">
          <ActionButton
            data-testid="shell.left-running-close"
            data-dialog-initial-focus="true"
            onClick={close}
          >
            Close
          </ActionButton>
        </footer>
      </div>
    </>
  );
}
