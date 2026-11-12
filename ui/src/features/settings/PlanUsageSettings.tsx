/** Monthly allowance in Usage and plan. Uses the plan usage store, existing facts, progress and Button.
 * Mounted only by the optional UI switch; never retains another key's or session's figures.
 */
import { useEffect, useState } from "react";
import { Button } from "../../components/Controls";
import { clockText } from "../../state/account";
import { useStore } from "../../state/createStore";
import { sessionForPick } from "../../state/modelChoice";
import { createPlanUsage, planPercent } from "../../state/planUsage";
import { useKeyUi } from "../setup/useKeys";
import { useApp } from "../shell/AppContext";
import { useCoreState, useShellState } from "../shell/hooks";
import { KeyNotice, useKeyReadiness } from "../shell/KeyNotice";

/** Existing usage section presentations, with an explicit refresh rather than background polling. */
export function PlanUsageSettings() {
  const { core } = useApp();
  const running = useCoreState((s) => s.server.phase === "running");
  const selected = useShellState((s) => s.selected);
  const via = core === null ? null : sessionForPick(core, selected);
  const itemId = useKeyUi((s) => s.status?.keyItemId ?? null);
  const blocked = useKeyReadiness() !== null;
  const [ledger] = useState(createPlanUsage);
  useEffect(
    () =>
      core === null || !running || via === null || blocked
        ? undefined
        : ledger.follow(core, via, itemId),
    [core, running, via, blocked, itemId, ledger],
  );
  const reading = useStore(ledger.store, (s) => s);
  const value = reading?.sessionId === via && reading.itemId === itemId ? reading.value : null;
  if (core === null || !running) return null;
  return (
    <section aria-labelledby="plan-usage-title" data-setting="plan-usage">
      <h3 id="plan-usage-title">This month</h3>
      <KeyNotice inSettings />
      {!blocked && via === null && <p className="key-line">Open a chat to read your plan usage.</p>}
      {!blocked && via !== null && (
        <>
          {(value === null || value.kind === "loading") && (
            <p className="key-line" role="status">
              Reading plan usage…
            </p>
          )}
          {value?.kind === "unavailable" && (
            <p className="key-line" role="status">
              Not available on this plan
            </p>
          )}
          {value?.kind === "notReported" && (
            <p className="key-line" role="status">
              Not reported
            </p>
          )}
          {value?.kind === "error" && (
            <p className="key-problem" role="alert">
              {value.message}
            </p>
          )}
          {value?.kind === "ready" && (
            <>
              <dl className="set-facts" data-testid="plan-usage-facts">
                <div className="check">
                  <dt>Used / limit</dt>
                  <dd className="mono">{planPercent(value.used)} / 100%</dd>
                </div>
                <div className="check">
                  <dt>Budget left</dt>
                  <dd className="mono">{planPercent(value.left)}</dd>
                </div>
                <div className="check">
                  <dt>Pay as you go</dt>
                  <dd>{value.payg === null ? "Not reported" : value.payg ? "On" : "Off"}</dd>
                </div>
                {value.resetAt !== null && (
                  <div className="check">
                    <dt>Resets</dt>
                    <dd>
                      {new Date(value.resetAt).toLocaleString(undefined, {
                        dateStyle: "medium",
                        timeStyle: "short",
                      })}
                    </dd>
                  </div>
                )}
                <div className="check">
                  <dt>Read at</dt>
                  <dd className="mono">{clockText(value.observedAt)}</dd>
                </div>
              </dl>
              <progress
                className="usage-bar"
                max={100}
                value={Math.min(100, value.used)}
                aria-label="Monthly plan usage"
                aria-valuetext={`${planPercent(value.used)} used, ${planPercent(value.left)} left`}
              />
            </>
          )}
          <Button
            data-testid="settings.plan-usage-refresh"
            type="button"
            shape="outlined"
            disabled={value === null || value.kind === "loading"}
            onClick={() => {
              void ledger.refresh();
            }}
          >
            Refresh usage
          </Button>
        </>
      )}
    </section>
  );
}
