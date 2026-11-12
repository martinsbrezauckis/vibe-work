/** Owned Firefox window and optional sharing add-on states. Uses confirmed core facts in Browser Settings. */
import { TEXT } from "../../../vocabulary";
import { Button, SettingsRow } from "../../../components/Controls";
import type { ChromeControlState } from "./useChromeControl";

export function FirefoxConnection({ chrome }: { chrome: ChromeControlState }) {
  const view = chrome.view;
  if (view === null) return null;
  const busy = chrome.busy !== null;
  const sharing = view.firefoxSharing;
  return (
    <>
      <SettingsRow data-setting="firefox-control-status">
        <span>
          Connection
          <span className="d" role="status">
            {view.setup.pairing}
          </span>
        </span>
        <Button
          data-testid="settings.firefox-window"
          shape="outlined"
          disabled={
            busy ||
            view.setup.pairingState === "blocked" ||
            !view.setup.package ||
            view.setup.pairingState === "notInstalled" ||
            view.setup.pairingState === "connecting"
          }
          onClick={() => {
            void chrome.run(
              !view.enabled
                ? { op: "setEnabled", enabled: true }
                : { op: view.setup.pairingState === "windowOpen" ? "closeFirefox" : "openFirefox" },
              "window",
            );
          }}
        >
          {view.setup.pairingState === "connecting"
            ? "Starting…"
            : !view.enabled
              ? "Turn on"
              : view.setup.pairingState === "windowOpen"
                ? "Close"
                : "Open window"}
        </Button>
      </SettingsRow>
      <p className="key-line">Other programs on this computer can also control this Firefox.</p>
      <p className="key-line">
        It starts empty each time you choose Open window: sign-ins there end when you close it.
      </p>
      {sharing ? (
        <SettingsRow data-setting="firefox-sharing">
          <span>
            Ask Vibe Work
            <span className="d" role="status">
              {sharing.label}
            </span>
          </span>
          <Button
            data-testid="settings.firefox-sharing"
            shape="outlined"
            disabled={busy || !view.setup.package}
            onClick={() => {
              void chrome.connect();
            }}
          >
            {sharing.state === "connected" ? "Add-ons" : TEXT.connectorConnect}
          </Button>
        </SettingsRow>
      ) : null}
    </>
  );
}
