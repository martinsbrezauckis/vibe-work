/** General's power card and immediate idle-sleep switch. Depends on the typed core state hook; SettingsPage renders this row only. */
import { CloseToTraySettings } from "./CloseToTraySettings";
import { SettingsRow, Switch } from "../../components/Controls";
import { useKeepAwake } from "../../state/useKeepAwake";
import { useApp } from "../shell/AppContext";

/** Automatic idle-sleep inhibition, with the approved inline OS refusal. */
export function KeepAwakeSettings() {
  const { core } = useApp();
  const { view, pending, error, change } = useKeepAwake(core);
  if (core === null) return null;
  return (
    <section aria-labelledby="keep-awake-title">
      <h3 id="keep-awake-title">Power</h3>
      <CloseToTraySettings />
      <SettingsRow data-setting="keep-awake">
        <span>
          <label htmlFor="keep-awake-switch">Keep this computer awake while Vibe is working</label>
        </span>
        <Switch
          data-testid="settings.keep-awake-settings-enabled"
          id="keep-awake-switch"
          aria-checked={view?.enabled ?? true}
          disabled={view === null || pending}
          onClick={() => {
            void change(!(view?.enabled ?? true));
          }}
        />
      </SettingsRow>
      {view?.enabled === true && view.status === "refused" && (
        <p className="file-note" role="status">
          This computer did not accept the request, so it may still sleep.
        </p>
      )}
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
