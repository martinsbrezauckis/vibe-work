/**
 * Browser delegation controls in Settings › Browser.
 * Depends on: the typed browser command and AppContext. Called by: SettingsPage.
 * Invariant: controls show only permissions confirmed by the core; a failed save leaves them unchanged.
 */
import { type ReactElement, useEffect, useState } from "react";

import { Selector } from "../../components/Selector";
import type { BrowserPermissions } from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import { term, TEXT } from "../../vocabulary";
import { useApp } from "../shell/AppContext";

const OFF: BrowserPermissions = { closeUserTabs: false };

/** Optional task-scoped authority for routine browser controls. */
export function BrowserDelegationSettings(): ReactElement | null {
  const { core, shell } = useApp();
  const [permissions, setPermissions] = useState<BrowserPermissions | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (core === null) return;
    let live = true;
    void core
      .run({ type: "browser", request: { op: "permissions" } })
      .then((response) => {
        if (!live) return;
        if (response.type !== "browser" || response.result.type !== "permissions")
          throw new Error("Unexpected browser permissions response.");
        setPermissions(response.result.permissions);
        setError(null);
      })
      .catch((thrown: unknown) => {
        if (live) setError(toCallError(thrown).error.message);
      });
    return () => {
      live = false;
    };
  }, [core]);

  const save = (next: BrowserPermissions) => {
    if (core === null || busy || permissions === null) return;
    setBusy(true);
    setError(null);
    void core
      .run({ type: "browser", request: { op: "setPermissions", permissions: next } })
      .then((response) => {
        if (response.type !== "browser" || response.result.type !== "permissions")
          throw new Error("Unexpected browser permissions response.");
        setPermissions(response.result.permissions);
      })
      .catch((thrown: unknown) => {
        setError(toCallError(thrown).error.message);
      })
      .finally(() => {
        setBusy(false);
      });
  };

  if (core === null) return null;
  const confirmed = permissions ?? OFF;
  return (
    <section aria-labelledby="browser-delegation-title">
      <h3 id="browser-delegation-title">{TEXT.browserDelegationTitle}</h3>
      {permissions === null && error === null && <p role="status">Reading browser permissions…</p>}
      {error !== null && (
        <p className="cbad" role="alert">
          {error}
        </p>
      )}
      <div className="check" data-setting="browser-close-user-tabs">
        <span title={term("browserPermissions", "closeUserTabs").tooltip}>
          {term("browserPermissions", "closeUserTabs").name}
        </span>
        <Selector<"ask" | "allow">
          label={term("browserPermissions", "closeUserTabs").name}
          title={
            term("browserPermissions", confirmed.closeUserTabs ? "closeAllow" : "closeAsk").tooltip
          }
          value={confirmed.closeUserTabs ? "allow" : "ask"}
          disabled={permissions === null || busy}
          options={[
            {
              value: "ask",
              label: term("browserPermissions", "closeAsk").name,
              tooltip: term("browserPermissions", "closeAsk").tooltip,
            },
            {
              value: "allow",
              label: term("browserPermissions", "closeAllow").name,
              tooltip: term("browserPermissions", "closeAllow").tooltip,
            },
          ]}
          onChange={(value) => {
            save({ ...confirmed, closeUserTabs: value === "allow" });
          }}
        />
      </div>
      <p className="browser-delegation-hint">{TEXT.browserDelegationHint}</p>
      <div className="browser-secret-permissions" data-setting="browser-secret-permissions">
        <div>
          <h4>{TEXT.secretPermissionsTitle}</h4>
          <p>{TEXT.secretPermissionsSummary}</p>
        </div>
        <button
          type="button"
          className="tb-btn"
          onClick={() => {
            shell.actions.openSettings("secrets");
          }}
        >
          {TEXT.secretPermissionsOpenVault}
        </button>
      </div>
    </section>
  );
}
