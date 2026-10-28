/**
 * Settings › General, "Open files": save edited open files before a message goes to Vibe (default on).
 * Depends on: AppContext, shell hooks, Controls. Called by: SettingsPage.
 * Invariants: a per-computer choice kept in the shell store; it applies from the next message.
 */
import type { ReactElement } from "react";
import { Switch } from "../../components/Controls";
import { useApp } from "../shell/AppContext";
import { useShellState } from "../shell/hooks";

/** The section. */
export function SaveBeforeSendSetting(): ReactElement {
  const { shell } = useApp();
  const on = useShellState((s) => s.saveBeforeSend);
  return (
    <section aria-labelledby="save-before-send-title">
      <h3 id="save-before-send-title">Open files</h3>
      <div className="check" data-setting="save-before-send">
        <span>Save open files before Vibe works</span>
        <Switch
          data-testid="settings.save-before-send"
          aria-label="Save open files before Vibe works"
          aria-checked={on}
          onClick={() => {
            shell.actions.setSaveBeforeSend(!on);
          }}
        />
      </div>
    </section>
  );
}
