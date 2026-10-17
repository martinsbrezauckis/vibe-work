/**
 * Settings, account and usage: `part="account"` (Settings › Account) shows the plan
 * and key type in plain words and who the key belongs to; `part="usage"` (Settings › Usage and plan) shows the open
 * conversation's tokens and context fill, and the per-minute rate limit.
 * Depends on: AppContext, hooks, state/account, state/modelChoice. Called by: SettingsPage.
 * Invariants: only what Vibe reports is shown. There is no plan quota (Mistral does not share it).
 * The rate headroom comes from the model proxy's view of Mistral's
 * answers: shown once an answer reported it, as "last seen" after a minute, nothing before.
 * The screen says so for the quota instead of leaving a blank. Context fill is "of N tokens until Vibe
 * compacts the conversation".
 */
import { TEXT } from "../../vocabulary";
import { useEffect, useState } from "react";

import {
  type AccountView,
  contextReadout,
  formatTokens,
  planInWords,
  rateLimitsText,
  readAccount,
  usageOf,
} from "../../state/account";
import { sessionForPick } from "../../state/modelChoice";
import { useApp } from "../shell/AppContext";
import { KeyNotice, useKeyReadiness } from "../shell/KeyNotice";
import { useCoreState, useShellState } from "../shell/hooks";

const STATUS_TEXT: Record<string, string> = {
  ready: "Connected",
  missing_key: "No key is set up yet.",
  unauthorized: "Mistral did not accept the key.",
  unavailable: "Vibe could not reach Mistral to read the account.",
};

/** The section (renders nothing without a running Vibe). */
export function AccountSettings({ part }: { part: "account" | "usage" }) {
  const { core } = useApp();
  const keyBlocked = useKeyReadiness() !== null;
  const running = useCoreState((s) => s.server.phase === "running");
  const selected = useShellState((s) => s.selected);
  const via = core === null ? null : sessionForPick(core, selected);
  const session = useCoreState((s) =>
    selected === null ? undefined : s.sessions[selected]?.session,
  );
  const [account, setAccount] = useState<AccountView | null>(null);
  const runtime = useCoreState((s) =>
    selected === null ? undefined : s.sessions[selected]?.runtime,
  );
  const limits = useCoreState((s) => s.rateLimits);
  const [now, setNow] = useState(() => Date.now());
  // Re-evaluate staleness while the section is open (the numbers are per minute).
  useEffect(() => {
    const timer = setInterval(() => {
      setNow(Date.now());
    }, 5_000);
    return () => {
      clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    if (part !== "account") return;
    if (core === null || !running || via === null || keyBlocked) return;
    let live = true;
    void readAccount(core, via).then((view) => {
      if (live) setAccount(view);
    });
    return () => {
      live = false;
    };
  }, [core, running, via, keyBlocked, part]);

  if (core === null || !running) return null;
  const usage = usageOf(session);
  const readout = contextReadout(usage, runtime);

  if (part === "account")
    return (
      <section aria-labelledby="account-settings-title" data-setting="account-facts">
        <h3 id="account-settings-title">Your account</h3>
        <KeyNotice
          inSettings
          message="Your Mistral key lets Vibe Work read your account details from Mistral."
        />
        {via === null && (
          <p className="key-line">Needs a session: Vibe reads the account through one.</p>
        )}
        {via !== null && account === null && !keyBlocked && (
          <p className="key-line">Reading the account...</p>
        )}
        {account !== null && !keyBlocked && (
          <dl className="set-facts" data-testid="account-facts">
            <div className="check">
              <dt>Plan</dt>
              <dd>{planInWords(account) ?? "Not reported"}</dd>
            </div>
            <div className="check">
              <dt>Account</dt>
              <dd>
                {account.problem === null
                  ? (STATUS_TEXT[account.status] ?? "Not reported")
                  : `Could not be read: ${account.problem}`}
              </dd>
            </div>
            {account.name !== null && (
              <div className="check">
                <dt>Name</dt>
                <dd>{account.name}</dd>
              </div>
            )}
            {(account.organization ?? account.workspace) !== null && (
              <div className="check">
                <dt>Workspace</dt>
                <dd>
                  {[account.organization, account.workspace].filter((x) => x !== null).join(" · ")}
                </dd>
              </div>
            )}
          </dl>
        )}
      </section>
    );

  return (
    <section aria-labelledby="usage-settings-title" data-setting="session-usage">
      <h3 id="usage-settings-title">{TEXT.thisSession}</h3>
      {selected === null ? (
        <p className="key-line">Open a session to see its usage.</p>
      ) : (
        <dl className="set-facts" data-testid="usage-facts">
          <div className="check">
            <dt>Tokens used</dt>
            {usage.totalTokens === null ? (
              <dd>Not reported yet</dd>
            ) : (
              <dd className="mono">
                {`${formatTokens(usage.totalTokens)} (${formatTokens(usage.inputTokens)} in, ${formatTokens(usage.outputTokens)} out)`}
              </dd>
            )}
          </div>
          <div className="check">
            <dt>Context</dt>
            <dd>{readout === null ? "Not reported yet" : readout.explain}</dd>
          </div>
        </dl>
      )}
      {/* the per-minute headroom Mistral's last answer reported (not per conversation). */}
      {rateLimitsText(limits, now) !== null && (
        <dl className="set-facts" data-testid="rate-limits">
          <div className="check">
            <dt>Rate limit this minute</dt>
            <dd className="mono">{rateLimitsText(limits, now)}</dd>
          </div>
        </dl>
      )}
      {readout !== null && (
        <progress
          className="usage-bar"
          max={100}
          value={readout.percent}
          aria-label={`Context ${String(readout.percent)} percent full`}
        />
      )}
      <p className="key-field-hint ui-help">
        Mistral shows how much of your plan is left on its website, not to apps (see the links
        below). Vibe compacts the session on its own before the context is full.
      </p>
    </section>
  );
}
