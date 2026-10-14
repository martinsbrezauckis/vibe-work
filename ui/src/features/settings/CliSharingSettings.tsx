/**
 * Settings, "Vibe command line": the switch "Keep Vibe Work sessions separate from the CLI" and
 * the one-line warning when the CLI's Vibe version differs from the bundled one.
 * Depends on: AppContext, protocol/client (`cliSharing`). Called by: settings/SettingsPage (the Settings dialog is gone).
 * Invariants: the switch is ON (sharing) by default, so "keep separate" is shown as the inverse of the
 * saved `share` flag; a change applies at the next start and the row says so; the help line names the side
 * effect (Vibe writes `logs/mcp-descriptors/` and plugin packages into the shared folder); the warning
 * shows only when both versions are known and differ; nothing is shown outside the app window.
 */
import { useCallback, useEffect, useState } from "react";

import type { CliSharingView } from "../../protocol/client/types";
import { toCallError } from "../../protocol/client/types";
import { useApp } from "../shell/AppContext";

/** The section (renders nothing in browser dev mode, where the host cannot read or save it). */
export function CliSharingSettings() {
  const { core, toasts } = useApp();
  const available = core !== null && core.client.kind !== "debug-api";
  const [view, setView] = useState<CliSharingView | null>(null);

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
    if (core === null || view === null) return;
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
      });
  }, [core, view, toasts]);

  if (!available) return null;
  return (
    <section aria-labelledby="cli-sharing-title">
      <h3 id="cli-sharing-title">Vibe command line</h3>
      <div className="check" data-setting="cli-sharing">
        <span>
          Keep Vibe Work sessions separate from the CLI
          <span className="d">
            Off (default): Vibe Work and the Vibe command line share one session list. Vibe then
            also writes its <span className="mono">logs/mcp-descriptors/</span> and plugin packages
            into the shared folder. On: Vibe Work keeps its own sessions. Applies the next time Vibe
            Work starts.
          </span>
          {view !== null && !view.cliHomeFound && (
            <span className="d" data-testid="cli-sharing-nohome">
              No Vibe command line folder was found on this computer.
            </span>
          )}
        </span>
        <button
          type="button"
          className="switch"
          role="switch"
          aria-label="Keep Vibe Work sessions separate from the CLI"
          aria-checked={view !== null && !view.share}
          disabled={view === null}
          onClick={flip}
        />
      </div>
      {view?.versionMismatch === true && (
        <p className="key-field-hint" role="status" data-testid="cli-version-warning">
          Your Vibe command line is {view.cliVersion}, Vibe Work uses {view.bundledVersion}.
          Sessions are shared, but a newer version may save them in a format the older one cannot
          open.
        </p>
      )}
    </section>
  );
}
