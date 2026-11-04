/** One General row, no helper text. Depends on the typed desktop state hook and existing controls. */
import { SettingsRow, Switch } from "../../components/Controls";
import { useCloseToTray } from "../../state/useCloseToTray";
import { detectMac } from "../../state/shortcuts";
import { TEXT } from "../../vocabulary";
import { useApp } from "../shell/AppContext";

/** Offered only while this run has a reachable tray. */
export function CloseToTraySettings() {
  const { core } = useApp();
  const { view, pending, error, change } = useCloseToTray(core?.client.desktop);
  if (view?.available !== true) return null;
  const label = detectMac() ? TEXT.closeToMenuBar : TEXT.closeToTray;
  return (
    <>
      <SettingsRow data-setting="close-to-tray">
        <label htmlFor="close-to-tray-switch">{label}</label>
        <Switch
          id="close-to-tray-switch"
          data-testid="settings.close-to-tray"
          aria-checked={view.enabled}
          disabled={pending}
          onClick={() => {
            void change(!view.enabled);
          }}
        />
      </SettingsRow>
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
    </>
  );
}
