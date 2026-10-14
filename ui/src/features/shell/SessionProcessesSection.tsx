/**
 * The open session's part of the Processes dialog: its dev servers and long commands, each with
 * its command, kind, ports, Output (live while shown) and Stop.
 * Depends on: AppContext, state/createStore, state/sessionProcesses. Called by: LeftRunning (the Processes dialog).
 * Invariants: renders only; output is text in a `<pre>` (never HTML); Stop never asks again (the click is the
 * decision, the window's own trusted surface).
 */
import type { ReactElement } from "react";

import { Button } from "../../components/Controls";
import type { SessionProcessView } from "../../protocol/core";
import { useStore } from "../../state/createStore";
import { type SessionProcesses, processTitle } from "../../state/sessionProcesses";

const KIND_TEXT: Record<string, string> = {
  vibe: "Vibe's background process: it ends when Vibe or this session's connection ends.",
  started:
    "Started for the agent by Vibe Work: it keeps running until you stop it, also after you quit.",
};

function Row({
  item,
  procs,
  output,
  stopping,
}: {
  item: SessionProcessView;
  procs: SessionProcesses;
  output: string | null | undefined;
  stopping: boolean;
}): ReactElement {
  const title = processTitle(item);
  const running = item.status === "running";
  return (
    <li data-session-process={item.id} data-kind={item.kind}>
      <div className="proc-row">
        <span className="left-running-what">
          <span className="mono proc-cmd" title={title}>
            {title}
          </span>
          <span className="d" title={KIND_TEXT[item.kind] ?? ""}>
            {item.kind === "started" ? "started for the agent" : "Vibe background process"}
            {` · ${item.status}`}
            {item.ports.length > 0 &&
              ` · port${item.ports.length === 1 ? "" : "s"} ${item.ports.join(", ")}`}
            {item.pid !== null && ` · pid ${String(item.pid)}`}
            {item.earlier && " · from last time"}
          </span>
        </span>
        <Button
          aria-expanded={output !== undefined}
          aria-label={`${output === undefined ? "Show" : "Hide"} the output of ${title}`}
          onClick={() => {
            procs.toggleOutput(item.id);
          }}
        >
          Output
        </Button>
        <Button
          variant="danger"
          small
          disabled={stopping || !running}
          aria-label={`Stop ${title}`}
          onClick={() => {
            void procs.stop(item.id);
          }}
        >
          {stopping ? "Stopping…" : "Stop"}
        </Button>
      </div>
      {output !== undefined && (
        <pre className="proc-output" aria-label={`Output of ${title}`} aria-live="off">
          {output ?? "Reading…"}
        </pre>
      )}
    </li>
  );
}

/** The section (a heading, the rows, or a line saying there is nothing). */
export function SessionProcessesSection({ procs }: { procs: SessionProcesses }): ReactElement {
  const state = useStore(procs.store, (s) => s);
  return (
    <section className="proc-section" data-testid="session-processes">
      <h3>This session</h3>
      {state.sessionId === null ? (
        <p className="d">Open a session to see its processes.</p>
      ) : state.items.length === 0 ? (
        <p className="d" data-testid="session-processes-empty">
          No dev servers or long commands are running for this session.
        </p>
      ) : (
        <ul className="left-running-list">
          {state.items.map((item) => (
            <Row
              key={item.id}
              item={item}
              procs={procs}
              output={item.id in state.outputs ? (state.outputs[item.id] ?? null) : undefined}
              stopping={state.stopping.includes(item.id)}
            />
          ))}
        </ul>
      )}
    </section>
  );
}
