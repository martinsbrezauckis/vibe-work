/**
 * Depends on: the typed host client and core state. Called by: SettingsPage.
 * Developer settings: Debug API enablement and its ordinary vault secret.
 * Uses the typed host client for metadata/regeneration and the core's user-only native clipboard command.
 * No token value enters React state, the bridge or Debug API UI replies.
 */
import { Button, IconButton, Switch } from "../../components/Controls";
import { type ReactElement, useEffect, useState } from "react";

import { toCallError } from "../../protocol/client/types";
import { useKeyUi } from "../setup/useKeys";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { TEXT } from "../../vocabulary";

/** The Developer section. */
export function DeveloperSettings(): ReactElement {
  const { core, toasts } = useApp();
  const debugOn = useCoreState((s) => s.debugApi.enabled);
  const reason = useCoreState((s) => s.debugApi.reason);
  const debugRequested = debugOn || reason === "vault-unavailable";
  const vaultState = useKeyUi((s) => s.status?.vaultState);
  const canSwitch = core?.client.kind === "tauri";
  const [tokenState, setTokenState] = useState("Reading…");
  const [busy, setBusy] = useState(false);
  const [permissions, setPermissions] = useState<boolean | null>(null);
  const [permissionsBusy, setPermissionsBusy] = useState(false);
  useEffect(() => {
    if (core === null || !canSwitch) return;
    let live = true;
    void core.run({ type: "developerPermissions", enabled: null }).then(
      (result) => {
        if (live && result.type === "developerPermissions") setPermissions(result.enabled);
      },
      (thrown: unknown) => {
        if (live)
          toasts.push({
            kind: "error",
            bellKind: "failure",
            text: toCallError(thrown).error.message,
          });
      },
    );
    return () => {
      live = false;
    };
  }, [core, canSwitch, toasts]);
  const flipPermissions = async () => {
    if (core === null || permissions === null || permissionsBusy) return;
    setPermissionsBusy(true);
    try {
      const result = await core.run({ type: "developerPermissions", enabled: !permissions });
      if (result.type === "developerPermissions") setPermissions(result.enabled);
    } catch (thrown: unknown) {
      const error = toCallError(thrown).error;
      if (error.code !== "confirmationDeclined")
        toasts.push({ kind: "error", bellKind: "failure", text: error.message });
    } finally {
      setPermissionsBusy(false);
    }
  };
  useEffect(() => {
    if (core === null || !canSwitch) return;
    let live = true;
    // The lookup settles inside the tracked promise: a refusal (locked vault, or the Debug API
    // driving the window, which never reads vault data) is a shown state, not the error of the
    // action that opened Developer.
    void core
      .trackAction(
        core.client.debugApiTokenItem(false).then(
          (id) => (id === null ? "Not set" : "In vault"),
          () => "Vault unavailable" as const,
        ),
      )
      .then((state) => {
        if (live) {
          setTokenState(state);
        }
      });
    return () => {
      live = false;
    };
  }, [core, canSwitch, vaultState, debugOn]);
  const tokenAction = async (rotate: boolean) => {
    if (core === null) return;
    setBusy(true);
    try {
      const id = await core.client.debugApiTokenItem(rotate);
      setTokenState(id === null ? "Not set" : "In vault");
      if (id === null) return;
      if (!rotate) await core.run({ type: "secrets", request: { op: "copy", id } });
      toasts.push({
        kind: "info",
        bellKind: "security",
        text: rotate ? "Token regenerated" : "Token copied",
      });
    } catch (thrown: unknown) {
      toasts.push({ kind: "error", bellKind: "failure", text: toCallError(thrown).error.message });
    } finally {
      setBusy(false);
    }
  };
  const flip = () => {
    if (core === null) return;
    core.client.setDebugApi(!debugRequested).catch((thrown: unknown) => {
      toasts.push({ kind: "error", bellKind: "failure", text: toCallError(thrown).error.message });
    });
  };
  return (
    <section aria-labelledby="developer-title">
      <h3 id="developer-title">Developer</h3>
      <div className="check" data-setting="developer-permissions">
        <span>
          {TEXT.developerPermissionsName}
          <span className="d">{permissions ? "On" : "Off"}</span>
        </span>
        <div className="set-actions">
          <Switch
            data-testid="settings.developer-permissions-switch"
            aria-label={TEXT.developerPermissionsName}
            aria-checked={permissions === true}
            disabled={!canSwitch || permissions === null || permissionsBusy}
            onClick={() => {
              void core?.trackAction(flipPermissions());
            }}
          />
        </div>
      </div>
      <div className="check" data-setting="debug-api">
        <span>
          Debug API
          <span className="d">
            {debugOn ? "On" : reason === "vault-unavailable" ? "Vault unavailable" : "Off"}
            {!canSwitch && " · App window only"}
          </span>
        </span>
        <div className="set-actions">
          <Switch
            data-testid="settings.developer-settings-switch"
            aria-label="Debug API"
            aria-checked={debugRequested}
            disabled={!canSwitch}
            onClick={flip}
          />
        </div>
      </div>
      {canSwitch && (
        <div className="check" data-setting="debug-token">
          <span>
            Debug API token
            <span className="d" data-testid="token-vault">
              {tokenState}
            </span>
          </span>
          <div className="set-actions">
            {debugOn && (
              <IconButton
                data-testid="settings.developer-copy-token"
                data-user-only="true"
                icon="copy"
                label="Copy token"
                title={vaultState === "unlocked" ? "Copy token" : "Unlock the vault to copy"}
                disabled={busy || vaultState !== "unlocked"}
                onClick={() => {
                  void core.trackAction(tokenAction(false));
                }}
              />
            )}
            <Button
              data-testid="settings.developer-settings-tb-btn"
              type="button"
              shape="outlined"
              disabled={busy}
              onClick={() => {
                void core.trackAction(tokenAction(true));
              }}
            >
              Regenerate token
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}
