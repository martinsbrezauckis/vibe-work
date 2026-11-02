/** Renders Vibe's spending/counters and native subagent shares from the session usage store in Settings. */
import { useEffect, useState } from "react";
import { formatTokens } from "../../state/account";
import { costText, sessionUsageOf, createSubagentUsage } from "../../state/sessionUsage";
import { useStore } from "../../state/createStore";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";

/** Session-owned counters, with subagents priced separately by Vibe. */
export function SessionUsageFacts({ sessionId }: { sessionId: string }) {
  const { core } = useApp();
  const entry = useCoreState((s) => s.sessions[sessionId]);
  const usage = sessionUsageOf(entry?.session, entry?.runtime);
  const [ledger] = useState(createSubagentUsage);
  useEffect(
    () => (core === null ? undefined : ledger.follow(core, sessionId)),
    [core, ledger, sessionId],
  );
  const current = useStore(ledger.store, (reading) =>
    reading?.sessionId === sessionId ? reading : null,
  );
  return (
    <>
      <dl className="set-facts" data-testid="session-cost">
        <div className="check">
          <dt>Cost (USD)</dt>
          <dd className="mono">{costText(usage.cost)}</dd>
        </div>
        <div className="check">
          <dt>Steps</dt>
          <dd className="mono">{formatTokens(usage.steps)}</dd>
        </div>
        <div className="check">
          <dt>Cached tokens</dt>
          <dd className="mono">{formatTokens(usage.cached)}</dd>
        </div>
        <div className="check">
          <dt>Tool calls</dt>
          <dd className="mono">{formatTokens(usage.tools)}</dd>
        </div>
      </dl>
      {(current?.rows.length ?? 0) > 0 && (
        <>
          <h4>Subagents</h4>
          {current?.rows.map((row) => (
            <dl className="set-facts" key={row.id}>
              <div className="check">
                <dt>{row.name}</dt>
                <dd className="mono">
                  {costText(row.cost)}
                  {row.estimated ? " (estimated)" : ""}
                </dd>
              </div>
              <div className="check">
                <dt>Steps</dt>
                <dd className="mono">{formatTokens(row.steps)}</dd>
              </div>
              <div className="check">
                <dt>Cached tokens</dt>
                <dd className="mono">{formatTokens(row.cached)}</dd>
              </div>
            </dl>
          ))}
        </>
      )}
      {current?.problem && (
        <p className="key-problem" role="alert">
          Subagent usage: {current.problem}
        </p>
      )}
    </>
  );
}
