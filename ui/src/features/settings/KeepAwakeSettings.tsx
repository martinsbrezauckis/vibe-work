/** General's power card and immediate idle-sleep switch. Depends on the typed core state hook; SettingsPage renders this row only. */
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
      <SettingsRow data-setting="keep-awake">
        <span>
          <label htmlFor="keep-awake-switch">Keep this computer awake while Vibe is working</label>
          <span className="d" id="keep-awake-description">
            While a chat is working, your computer does not go to sleep on its own.
          </span>
        </span>
        <Switch
          data-testid="settings.keep-awake-settings-enabled"
          id="keep-awake-switch"
          aria-describedby="keep-awake-description"
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
