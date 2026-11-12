/**
 * Settings, "Privacy": Vibe's usage telemetry and experiments switches, and where to
 * change what Mistral may do with your conversations (that is an account setting, not this one).
 * Depends on: AppContext, hooks, state/privacy, state/modelChoice. Called by: settings/SettingsPage (the Settings dialog is gone).
 * Invariants: the consequence line is always visible: with telemetry off Mistral's experiments do not
 * run, so features still being rolled out (smart approve, extra models) do not appear. Turning a switch
 * on asks the native confirmation (the gate requires it); off does not. A value that the environment
 * or an administrator sets is shown as such and cannot be changed here.
 */
import { Switch } from "../../components/Controls";
import { useEffect, useState } from "react";

import { sessionForPick } from "../../state/modelChoice";
import {
  type PrivacyView,
  type SwitchName,
  type SwitchView,
  readPrivacy,
  writePrivacy,
} from "../../state/privacy";
import { useApp } from "../shell/AppContext";
import { useCoreState, useShellState } from "../shell/hooks";

const SETTERS: Record<string, string> = {
  environment: "Vibe Work for this run",
  admin: "your administrator",
  overrides: "a setting of this chat",
};

/** The section (renders nothing without a running Vibe). */
export function PrivacySettings() {
  const { core } = useApp();
  const running = useCoreState((s) => s.server.phase === "running");
  const selected = useShellState((s) => s.selected);
  const via = core === null ? null : sessionForPick(core, selected);
  const [view, setView] = useState<PrivacyView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState<SwitchName | null>(null);

  useEffect(() => {
    if (core === null || !running || via === null) return;
    let live = true;
    readPrivacy(core, via)
      .then((v) => {
        if (live) {
          setView(v);
          setError(null);
        }
      })
      .catch((thrown: unknown) => {
        if (live)
          setError(
            `Could not read privacy settings: ${thrown instanceof Error ? thrown.message.replace(/\.$/, "") : "No error details were returned"}. Try again.`,
          );
      });
    return () => {
      live = false;
    };
  }, [core, running, via]);

  if (core === null || !running) return null;

  const flip = (which: SwitchName, current: SwitchView) => {
    if (via === null || current.on === null) return;
    setSaving(which);
    setError(null);
    writePrivacy(core, via, which, !current.on)
      .then(() => readPrivacy(core, via))
      .then((v) => {
        setView(v);
      })
      .catch((thrown: unknown) => {
        setError(
          `Could not update privacy settings: ${thrown instanceof Error ? thrown.message.replace(/\.$/, "") : "No error details were returned"}. Try again.`,
        );
      })
      .finally(() => {
        setSaving(null);
      });
  };

  const row = (which: SwitchName, label: string, detail: string) => {
    const current = view?.[which];
    const lockedBy = current?.setBy ?? null;
    return (
      <div className="check" data-setting={which === "telemetry" ? "usage-data" : "experiments"}>
        <span>
          {label}
          <span className="d">{detail}</span>
          {lockedBy !== null && (
            <span className="d" data-testid={`privacy-${which}-lock`}>
              Set by {SETTERS[lockedBy] ?? lockedBy}; it cannot be changed here.
            </span>
          )}
        </span>
        <Switch
          data-testid="settings.privacy-settings-flip"
          aria-label={label}
          aria-checked={current?.on === true}
          disabled={
            current === undefined || current.on === null || lockedBy !== null || saving !== null
          }
          onClick={() => {
            if (current !== undefined) flip(which, current);
          }}
        />
      </div>
    );
  };

  return (
    <section aria-labelledby="privacy-settings-title">
      <h3 id="privacy-settings-title">Privacy</h3>
      {via === null && (
        <p className="key-line">Needs a chat: Vibe reads and saves these switches through one.</p>
      )}
      {via !== null && view === null && error === null && (
        <p className="key-line">Reading the switches…</p>
      )}
      {view !== null && (
        <>
          {row(
            "telemetry",
            "Send usage data to Mistral",
            "Sends anonymous usage, errors and timings to Mistral.",
          )}
          {row("experiments", "Take part in Mistral's experiments", "Needs usage data on.")}
          <p className="key-field-hint ui-help" data-testid="privacy-consequence">
            Off: features still rolling out (smart approve, some models) are not offered.
          </p>
        </>
      )}
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
