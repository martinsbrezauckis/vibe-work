/**
 * The notification section of Settings: which events produce an OS notification, and whether the
 * system lets the app show them. Notifications appear only while the window is not focused or the
 * session is not the open one; supported activation opens the session and its full notification.
 * Depends on: AppContext, state/osNotify, state/createStore, components/Selector. Called by: settings/SettingsPage (the Settings dialog is gone).
 * Invariants: the level is remembered (localStorage via the notifier); when the host reports that
 * the OS refused the last notification, the row says "off in system settings" instead of pretending.
 */
import { Selector } from "../../components/Selector";
import { NOTIFY_LEVELS, type NotifyLevel } from "../../state/osNotify";
import { useStore } from "../../state/createStore";
import { useApp } from "./AppContext";

const LABEL: Record<NotifyLevel, string> = {
  errors: "Errors",
  permissions: "Errors and permission requests",
  all: "Everything (also when a session finishes)",
};

/** The section. */
export function NotificationSettings() {
  const { notifier } = useApp();
  const level = useStore(notifier.store, (s) => s.level);
  const error = useStore(notifier.store, (s) => s.error);
  return (
    <section>
      <h3>Notifications</h3>
      <div className="check" data-setting="notification-level">
        <span>
          Tell me about
          <span className="d">
            Shown by the system when the window is in the background or another session is open.
            Clicking one opens the session and the full message. Use Notifications in the title bar
            to read earlier messages.
          </span>
        </span>
        <Selector
          testId="shell.notification-settings-set-level"
          label="Notification level"
          value={level}
          options={NOTIFY_LEVELS.map((l) => ({
            value: l,
            label: LABEL[l],
          }))}
          onChange={(l) => {
            notifier.setLevel(l);
          }}
        />
      </div>
      {error !== null && (
        <p className="key-field-hint ui-help" aria-live="polite">
          Off in system settings: allow notifications for Vibe Work in the system settings, then try
          again.
        </p>
      )}
    </section>
  );
}
