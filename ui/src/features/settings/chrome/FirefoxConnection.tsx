/** Owned Firefox window and optional sharing add-on states. Uses confirmed core facts in Browser Settings. */
import { TEXT, term } from "../../../vocabulary";
import { Button, SettingsRow } from "../../../components/Controls";
import { ACT_CONNECTING_ELLIPSIS, ACT_FIREFOX_ADDONS, ACT_FIREFOX_PAIR } from "../actionLabels";
import type { ChromeControlState } from "./useChromeControl";

/**
 * Firefox card rows: connection truth (the existing Use my Firefox switch controls availability), and the sharing add-on row
 * when the core reports one. Renders nothing before the core view arrives.
 */
export function FirefoxConnection({ chrome }: { chrome: ChromeControlState }) {
  const view = chrome.view;
  if (view === null) return null;
  const busy = chrome.busy !== null;
  const sharing = view.firefoxSharing;
  const waiting = sharing?.state === "notPaired";
  const changed = sharing?.state === "otherOwner";
  const pair = waiting;
  const connecting = sharing?.state === "connecting";
  return (
    <>
      <SettingsRow data-setting="firefox-control-status">
        <span>
          Connection
          <span className="d" role="status" aria-live="polite">
            {!view.enabled
              ? "Off"
              : view.setup.pairingState === "windowClosed" && view.setup.package
                ? "Ready"
                : view.setup.pairing}
          </span>
        </span>
      </SettingsRow>
      <p className="key-line">Other programs on this computer can also control this Firefox.</p>
      <p className="key-line">
        When Vibe Work starts Firefox, it uses an empty profile. Sign-ins there end when that
        Firefox closes.
      </p>
      {sharing ? (
        <SettingsRow data-setting="firefox-sharing">
          <span>
            Ask Vibe Work
            <span className="d" role="status" aria-live="polite">
              {sharing.label}
            </span>
          </span>
          {!changed ? (
            <Button
              data-testid="settings.firefox-sharing"
              shape="outlined"
              disabled={busy || !view.setup.package || connecting}
              onClick={() => {
                if (pair) void chrome.run({ op: "pairFirefox" }, "pair");
                else void chrome.connect();
              }}
            >
              {sharing.state === "connected"
                ? ACT_FIREFOX_ADDONS
                : connecting
                  ? ACT_CONNECTING_ELLIPSIS
                  : waiting
                    ? ACT_FIREFOX_PAIR
                    : TEXT.connectorConnect}
            </Button>
          ) : null}
        </SettingsRow>
      ) : null}
      {waiting || changed ? (
        <>
          <p className="key-line">
            {changed
              ? `Firefox is paired with another Vibe Work. To change it, open ${term("workNames", "chromeExtension").name} from the Firefox toolbar and choose Pair with Vibe Work.`
              : "Firefox is ready. Pair it here to connect."}
          </p>
          {waiting && view.setup.fingerprint ? (
            <p className="key-line">
              Compare identity · <code>{view.setup.fingerprint}</code>
            </p>
          ) : null}
        </>
      ) : sharing &&
        ["notSetUp", "extensionNotInstalled", "notConnected"].includes(sharing.state) ? (
        <p className="key-line">Open Firefox. Add the extension if it is missing.</p>
      ) : null}
    </>
  );
}
