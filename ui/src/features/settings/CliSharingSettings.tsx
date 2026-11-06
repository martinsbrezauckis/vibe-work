/**
 * Settings, "Vibe command line": the switch "Keep Vibe Work sessions separate from the CLI" and
 * the one-line warning when the CLI's Vibe version differs from the bundled one.
 * Depends on: AppContext, protocol/client (`cliSharing`). Called by: settings/SettingsPage (the Settings dialog is gone).
 * Invariants: the switch is ON (sharing) by default, so "keep separate" is shown as the inverse of the
 * saved `share` flag; a change applies at the next start and the row says so; the warning
 * shows only when both versions are known and differ; nothing is shown outside the app window.
 */
import { Switch } from "../../components/Controls";
import { useCallback, useEffect, useState } from "react";

import type { CliSharingView } from "../../protocol/client/types";
import { toCallError } from "../../protocol/client/types";
import { useApp } from "../shell/AppContext";

/** The section (renders nothing in browser dev mode, where the host cannot read or save it). */
export function CliSharingSettings() {
  const { core, toasts } = useApp();
  const available = core !== null && core.client.kind !== "debug-api";
  const [view, setView] = useState<CliSharingView | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (core === null || !available) return;
    let live = true;
    core.client
      .cliSharing()
      .then((v) => {
        if (live) setView(v);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [core, available]);

  const flip = useCallback(() => {
    if (core === null || view === null || saving) return;
    setSaving(true);
    core.client
      .cliSharing(!view.share)
      .then((v) => {
        setView(v);
      })
      .catch((thrown: unknown) => {
        toasts.push({
          kind: "error",
          bellKind: "failure",
          text: toCallError(thrown).error.message,
        });
      })
      .finally(() => {
        setSaving(false);
      });
  }, [core, view, toasts, saving]);

  if (!available) return null;
  return (
    <section aria-labelledby="cli-sharing-title">
      <h3 id="cli-sharing-title">Vibe command line</h3>
      <div className="check" data-setting="cli-sharing">
        <span>
          Keep Vibe Work sessions separate from the CLI
          <span className="d">Applies after restart</span>
          {view !== null && !view.cliHomeFound && (
            <span className="d" data-testid="cli-sharing-nohome">
              Vibe command line folder not found
            </span>
          )}
        </span>
        <Switch
          data-testid="settings.cli-sharing-settings-switch"
          aria-label="Keep Vibe Work sessions separate from the CLI"
          aria-checked={view !== null && !view.share}
          disabled={view === null || saving}
          onClick={flip}
        />
      </div>
      {view?.versionMismatch === true && (
        <p className="key-field-hint ui-help" role="status" data-testid="cli-version-warning">
          Version mismatch: CLI {view.cliVersion} · Vibe Work {view.bundledVersion}
        </p>
      )}
    </section>
  );
}
