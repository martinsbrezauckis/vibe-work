/**
 * Settings > General > "Send to Vibe Work": one switch, "Right-click menus", that adds or removes the system's
 * right-click entries, with the status line of this system. Depends on the shared switch state (`state/sendTo`);
 * SettingsPage renders it. Hidden where the host has no such entries (browser dev mode, other systems).
 */
import { SettingsRow, Switch } from "../../components/Controls";
import { sendToDescription, sendToRequest, sendToStatus, useSendTo } from "../../state/sendTo";
import { useApp } from "../shell/AppContext";

/** The section; the switch reflects what is on the system now. */
export function SendToSettings() {
  const { core } = useApp();
  const client = core?.client ?? null;
  const { view, pending, error } = useSendTo(client);
  if (client === null || view === null || !view.supported) return null;
  const status = sendToStatus(view);
  return (
    <section aria-labelledby="send-to-title">
      <h3 id="send-to-title">Send to Vibe Work</h3>
      <SettingsRow data-setting="send-to">
        <span>
          <label htmlFor="send-to-switch">Right-click menus</label>
          <span className="d" id="send-to-description">
            {sendToDescription(view)}
          </span>
          {status !== null && (
            <span className="d" role="status">
              {status}
            </span>
          )}
        </span>
        <Switch
          data-testid="settings.send-to-enabled"
          id="send-to-switch"
          aria-describedby="send-to-description"
          aria-checked={view.on}
          disabled={pending}
          onClick={() => {
            void sendToRequest(client, { op: "set", on: !view.on });
          }}
        />
      </SettingsRow>
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
