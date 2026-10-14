/**
 * Provider sessions in Settings > Privacy and data, with a user-only native sign-in viewport.
 * Depends on: account commands, AppContext and NativeViewport. Called by: SettingsPage.
 * Invariant: sign-in uses direct user input; status is cookie presence, never proof of remote validity.
 */
import { useCallback, useEffect, useState } from "react";

import { NativeViewport } from "../../components/NativeViewport";
import type { Provider, ProviderAccount, Rect } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { ACCOUNT_PROVIDERS, accountCommand } from "./accountCommands";

/** Account controls and the provider sign-in page, when opened. */
export function SignedInAccounts() {
  const { core } = useApp();
  const [accounts, setAccounts] = useState<ProviderAccount[] | null>(null);
  const [signingIn, setSigningIn] = useState<Provider | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (core === null) return;
    const result = await accountCommand(core, { op: "accounts" });
    if (result.type === "accounts") setAccounts(result.accounts);
  }, [core]);

  const problem = useCallback((thrown: unknown) => {
    setError(thrown instanceof Error ? thrown.message : "Could not update the browser accounts.");
  }, []);

  useEffect(() => {
    let live = true;
    if (core !== null) {
      accountCommand(core, { op: "accounts" })
        .then((result) => {
          if (live && result.type === "accounts") setAccounts(result.accounts);
        })
        .catch((thrown: unknown) => {
          if (live) problem(thrown);
        });
    }
    return () => {
      live = false;
      if (core !== null) void accountCommand(core, { op: "closeSignIn" }).catch(() => undefined);
    };
  }, [core, problem]);

  const report = useCallback(
    (rect: Rect | null) => {
      if (core === null) return;
      void accountCommand(core, {
        op: "accountsLayout",
        ...(rect === null ? {} : { rect }),
      }).catch(problem);
    },
    [core, problem],
  );

  const change = async (provider: Provider, signOut: boolean) => {
    if (core === null) return;
    setBusy(true);
    setError(null);
    try {
      const result = await accountCommand(core, {
        op: signOut ? "signOut" : "signIn",
        provider,
      });
      if (result.type === "accounts") setAccounts(result.accounts);
      if (!signOut) setSigningIn(provider);
    } catch (thrown) {
      problem(thrown);
    } finally {
      setBusy(false);
    }
  };

  const finish = async () => {
    if (core === null) return;
    setBusy(true);
    try {
      await accountCommand(core, { op: "closeSignIn" });
      setSigningIn(null);
      await refresh();
    } catch (thrown) {
      problem(thrown);
    } finally {
      setBusy(false);
    }
  };

  const signingInLabel = ACCOUNT_PROVIDERS.find(({ provider }) => provider === signingIn)?.label;

  return (
    <section aria-labelledby="signed-in-accounts-title" data-setting="signed-in-accounts">
      <h3 id="signed-in-accounts-title">Signed-in accounts</h3>
      <p className="key-field-hint">
        Sign in once here. Your browser keeps the session so the agent can sign in to services on
        your behalf. Access beyond basic identity asks for your confirmation.
      </p>
      {core === null && <p className="key-line">Available in the desktop app.</p>}
      {ACCOUNT_PROVIDERS.map(({ provider, label }) => {
        const account = accounts?.find((item) => item.provider === provider);
        return (
          <div className="check account-provider" data-provider={provider} key={provider}>
            <span>
              {label}
              <span className="d" aria-live="polite">
                {account === undefined
                  ? "Session status not checked"
                  : account.present
                    ? "Session present"
                    : "No saved session"}
              </span>
            </span>
            <button
              type="button"
              className="tb-btn"
              disabled={core === null || busy}
              onClick={() => void change(provider, false)}
            >
              Sign in to {label}
            </button>
            <button
              type="button"
              className="tb-btn danger"
              aria-label={`Sign out of ${label}`}
              disabled={core === null || busy}
              onClick={() => void change(provider, true)}
            >
              Sign out
            </button>
          </div>
        );
      })}
      <p className="key-field-hint">
        Session presence comes from cookies. A session may have expired.
      </p>
      <div className="key-actions wrap">
        <button
          type="button"
          className="tb-btn"
          disabled={core === null || busy}
          onClick={() => void refresh().catch(problem)}
        >
          Refresh accounts
        </button>
      </div>
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
      {signingIn !== null && (
        <div className="account-sign-in">
          <p className="key-field-hint">Sign in by hand below, then select Done.</p>
          <div className="key-actions wrap">
            <button
              type="button"
              className="tb-btn primary"
              disabled={busy}
              onClick={() => void finish()}
            >
              Done
            </button>
          </div>
          <NativeViewport
            report={report}
            live
            label={`${signingInLabel ?? "Provider"} sign-in page`}
            className="account-sign-in-viewport"
          />
        </div>
      )}
    </section>
  );
}
