/** Browser availability and shared connection controls. Uses confirmed core state; called by BrowserSettings. */
import { type ReactElement, useState } from "react";
import { Button, SettingsRow, Switch } from "../../../components/Controls";
import { ChromeRemoveDialog } from "./ChromeDialogs";
import { ChromeConnection } from "./ChromeConnection";
import { chromeAvailable } from "./chromeRows";
import { useChromeControl } from "./useChromeControl";
import { term, TEXT, wording } from "../../../vocabulary";

/** Chrome, Edge and Safari share every connection step; Firefox owns a separate agent window. */
export function YourChromeSettings({
  browser = "Chrome",
}: {
  browser?: "Chrome" | "Edge" | "Safari" | "Firefox";
}): ReactElement | null {
  const chrome = useChromeControl(browser);
  const prefix = browser.toLowerCase();
  const firefox = browser === "Firefox";
  const useMyBrowser =
    browser === "Chrome"
      ? term("builtinSkills", "vw:chrome").name
      : wording(TEXT.browserUse, { browser });
  const [removeOpen, setRemoveOpen] = useState(false);
  const view = chrome.view;
  const removeRequired = view?.setup.addon?.action === "remove";
  const setUp = view !== null && view.setup.state !== "notSetUp";
  const busy = chrome.busy !== null || view === null;
  if (!chromeAvailable(view)) return null;
  return (
    <section aria-labelledby={`${prefix}-title`} data-setting={`${prefix}-control`}>
      <h3 id={`${prefix}-title`}>
        {firefox
          ? TEXT.firefoxName
          : browser === "Safari"
            ? TEXT.safariName
            : browser === "Edge"
              ? TEXT.edgeName
              : TEXT.chromeName}
      </h3>
      {!firefox &&
      view?.setup.browserIssue === undefined &&
      view?.setup.addon !== undefined &&
      (browser !== "Safari" || view.setup.addon.extensionVersion) ? (
        <p className="key-line">
          {view.setup.addon.extensionVersion
            ? `Extension ${view.setup.addon.extensionVersion}`
            : view.setup.installState === "store"
              ? "Extension version unknown"
              : "Extension not installed"}
        </p>
      ) : null}
      {firefox ? (
        <SettingsRow data-setting="firefox-control-status">
          <span>
            Connection
            <span className="d" role="status">
              {chrome.readError !== null && view === null
                ? "State unavailable"
                : view?.setup.pairing}
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
              if (view === null) return;
              void chrome.run(
                !view.enabled
                  ? { op: "setEnabled", enabled: true }
                  : {
                      op: view.setup.pairingState === "windowOpen" ? "closeFirefox" : "openFirefox",
                    },
                "window",
              );
            }}
          >
            {view?.setup.pairingState === "connecting"
              ? "Starting…"
              : view?.enabled === false
                ? "Turn on"
                : view?.setup.pairingState === "windowOpen"
                  ? "Close"
                  : "Open window"}
          </Button>
        </SettingsRow>
      ) : (
        <ChromeConnection chrome={chrome} browser={browser} showSwitch />
      )}
      {(setUp || browser === "Safari") && !firefox ? (
        <SettingsRow data-setting={`${prefix}-control-enabled`}>
          <span>{useMyBrowser}</span>
          <Switch
            data-testid={`settings.${prefix}-enabled`}
            aria-label={useMyBrowser}
            aria-checked={view?.enabled ?? false}
            disabled={busy || (browser === "Safari" && !view.setup.package)}
            onClick={() => {
              void chrome.run({ op: "setEnabled", enabled: !view?.enabled }, "setEnabled");
            }}
          />
        </SettingsRow>
      ) : null}
      {firefox ? (
        <p
          className={chrome.status?.alert === true ? "key-problem" : "key-line"}
          role={chrome.status?.alert === true ? "alert" : "status"}
          aria-live="polite"
          data-chrome-status=""
        >
          {chrome.status?.text ?? ""}
        </p>
      ) : null}
      {setUp && !firefox && !removeRequired ? (
        <SettingsRow data-setting={`${prefix}-control-remove`}>
          <span>Remove from Vibe Work</span>
          <Button
            data-testid={`settings.${prefix}-remove-open`}
            shape="outlined"
            variant="danger"
            onClick={() => {
              setRemoveOpen(true);
            }}
          >
            Remove…
          </Button>
        </SettingsRow>
      ) : null}
      <ChromeRemoveDialog
        open={removeOpen}
        chrome={chrome}
        browser={browser}
        onClose={() => {
          setRemoveOpen(false);
        }}
      />
    </section>
  );
}
