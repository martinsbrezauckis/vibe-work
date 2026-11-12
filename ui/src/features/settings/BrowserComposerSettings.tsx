/** Independent browser visibility and built-in availability rows. Core owns persistence; Browser Settings renders confirmed values. */
import { useState } from "react";
import { SettingsRow, Switch } from "../../components/Controls";
import type { BrowserRequest, ComposerBrowser } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";

export function BrowserComposerSettings({ browser }: { browser: ComposerBrowser }) {
  const app = useApp();
  const view = useCoreState((s) => s.chromeControl);
  const prefs = view?.browserPreferences;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const prefix = browser.toLowerCase();
  const change = (request: BrowserRequest) => {
    setBusy(true);
    setError(null);
    void app.core
      ?.run({ type: "browser", request }, { origin: "window" })
      .catch(() => {
        setError("Could not save the browser setting. Try again.");
      })
      .finally(() => {
        setBusy(false);
      });
  };
  return (
    <>
      {browser === "Builtin" && (
        <SettingsRow data-setting="builtin-control-enabled">
          <span>Use Vibe Work browser</span>
          <Switch
            data-testid="settings.builtin-enabled"
            aria-label="Use Vibe Work browser"
            aria-checked={prefs?.builtinEnabled ?? true}
            disabled={busy || view == null || prefs?.builtinAvailable === false}
            onClick={() => {
              change({ op: "setBuiltinEnabled", enabled: !(prefs?.builtinEnabled ?? true) });
            }}
          />
        </SettingsRow>
      )}
      <SettingsRow data-setting={`${prefix}-composer`}>
        <span>Show in composer</span>
        <Switch
          data-testid={`settings.${prefix}-composer`}
          aria-label="Show in composer"
          aria-checked={prefs?.shown.includes(browser) ?? true}
          disabled={busy || view == null}
          onClick={() => {
            change({
              op: "setComposerBrowser",
              browser,
              shown: !(prefs?.shown.includes(browser) ?? true),
            });
          }}
        />
      </SettingsRow>
      {error && (
        <p role="alert" className="key-problem">
          {error}
        </p>
      )}
    </>
  );
}
