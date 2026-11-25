/** Renders Vibe's spending/counters and native subagent shares from the session usage store in Settings. */
import { useEffect, useState, type ReactNode } from "react";
import { formatTokens } from "../../state/account";
import { costText, sessionUsageOf, createSubagentUsage } from "../../state/sessionUsage";
import { useStore } from "../../state/createStore";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";

/** A reported number keeps the tabular number font; a missing one reads as text, like "Not reported" elsewhere. */
function Value({ value, children }: { value: number | null; children: ReactNode }) {
  return <dd className={value === null ? undefined : "mono"}>{children}</dd>;
}

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
          <Value value={usage.cost}>{costText(usage.cost)}</Value>
        </div>
        <div className="check">
          <dt>Steps</dt>
          <Value value={usage.steps}>{formatTokens(usage.steps)}</Value>
        </div>
        <div className="check">
          <dt>Cached tokens</dt>
          <Value value={usage.cached}>{formatTokens(usage.cached)}</Value>
        </div>
        <div className="check">
          <dt>Tool calls</dt>
          <Value value={usage.tools}>{formatTokens(usage.tools)}</Value>
        </div>
      </dl>
      {(current?.rows.length ?? 0) > 0 && (
        <>
          <h4>Subagents</h4>
          {current?.rows.map((row) => (
            <dl className="set-facts" key={row.id}>
              <div className="check">
                <dt>{row.name}</dt>
                <Value value={row.cost}>
                  {costText(row.cost)}
                  {row.cost !== null && row.estimated ? " (estimated)" : ""}
                </Value>
              </div>
              <div className="check">
                <dt>Steps</dt>
                <Value value={row.steps}>{formatTokens(row.steps)}</Value>
              </div>
              <div className="check">
                <dt>Cached tokens</dt>
                <Value value={row.cached}>{formatTokens(row.cached)}</Value>
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
