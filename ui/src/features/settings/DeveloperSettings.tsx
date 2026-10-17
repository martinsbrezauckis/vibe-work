/**
 * Settings › Developer: the Debug API switch and its token file (moved from the old Settings dialog).
 * Depends on: AppContext, hooks, protocol/client types. Called by: SettingsPage.
 * Invariants: the switch calls the host (Tauri command) and the indicator follows the core's flag event, never the
 * click; in browser dev mode the switch is disabled with the reason; the token never crosses the bridge, only the
 * path of the file that holds it.
 */
import { Button } from "../../components/Controls";
import { PathLine } from "../../components/PathLine";
import { type ReactElement, useEffect, useState } from "react";

import { toCallError } from "../../protocol/client/types";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";

/** The Developer section. */
export function DeveloperSettings(): ReactElement {
  const { core, toasts } = useApp();
  const debugOn = useCoreState((s) => s.debugApi.enabled);
  const reason = useCoreState((s) => s.debugApi.reason);
  const canSwitch = core?.client.kind === "tauri";
  const [tokenFile, setTokenFile] = useState<string | null>(null);
  useEffect(() => {
    if (core === null || !canSwitch) return;
    let live = true;
    core.client
      .debugApiTokenFile(false)
      .then((path) => {
        if (live) setTokenFile(path);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [core, canSwitch]);
  const regenerate = () => {
    if (core === null) return;
    core.client
      .debugApiTokenFile(true)
      .then((path) => {
        setTokenFile(path);
        toasts.push({
          kind: "info",
          bellKind: "security",
          text: "New Debug API token written. The old one no longer works.",
        });
      })
      .catch((thrown: unknown) => {
        toasts.push({
          kind: "error",
          bellKind: "failure",
          text: toCallError(thrown).error.message,
        });
      });
  };
  const flip = () => {
    if (core === null) return;
    core.client.setDebugApi(!debugOn).catch((thrown: unknown) => {
      toasts.push({ kind: "error", bellKind: "failure", text: toCallError(thrown).error.message });
    });
  };
  return (
    <section aria-labelledby="developer-title">
      <h3 id="developer-title">Developer</h3>
      <div className="check" data-setting="debug-api">
        <span>
          Debug API
          <span className="d">
            Lets a local agent do what you can do in Vibe Work, terminal and files included, and
            answers the app&apos;s confirmations for you; only your vault stays yours. It hides
            known secrets where it can, but that is no guarantee. 127.0.0.1 only, with a token in a
            file only you can read. Off by default; developer builds start with it on. While it
            runs, the status bar says &quot;Debug API&quot; (developer builds also show a badge in
            the title bar).
            {debugOn && ` Now on (${reason}).`}
            {!canSwitch && " The switch works in the app window."}
          </span>
        </span>
        <button
          data-testid="settings.developer-settings-switch"
          type="button"
          className="switch"
          role="switch"
          aria-label="Debug API"
          aria-checked={debugOn}
          disabled={!canSwitch}
          onClick={flip}
        />
      </div>
      {canSwitch && (
        <div className="check" data-setting="debug-token">
          <span>
            Token file
            <span className="d" data-testid="token-file">
              {tokenFile === null ? "…" : <PathLine path={tokenFile} />}
            </span>
            <span className="d">
              Agents read the token from this file. It stays the same across restarts until you
              regenerate it.
            </span>
          </span>
          <Button
            data-testid="settings.developer-settings-tb-btn"
            type="button"
            small
            onClick={regenerate}
          >
            Regenerate token
          </Button>
        </div>
      )}
    </section>
  );
}
