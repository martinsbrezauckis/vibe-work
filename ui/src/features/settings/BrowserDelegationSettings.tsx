/** Vault navigation from Browser settings. */
import { Button } from "../../components/Controls";
import { TEXT } from "../../vocabulary";
import { useApp } from "../shell/AppContext";
export function BrowserDelegationSettings() {
  const { core, shell } = useApp();
  if (core === null) return null;
  return (
    <div className="check set-row" data-setting="browser-secret-permissions">
      <span>{TEXT.secretPermissionsTitle}</span>
      <Button
        data-testid="settings.browser-delegation-settings-open-settings"
        type="button"
        shape="outlined"
        onClick={() => {
          shell.actions.openSettings("secrets");
        }}
      >
        {TEXT.secretPermissionsOpenVault}
      </Button>
    </div>
  );
}
