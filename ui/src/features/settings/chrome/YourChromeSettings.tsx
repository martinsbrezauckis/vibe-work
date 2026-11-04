/** Browser availability and connection controls. Uses the core's confirmed state; called by BrowserSettings. */
import { type ReactElement, useState } from "react";
import { Button, SettingsRow, Switch } from "../../../components/Controls";
import { ChromeRemoveDialog, ChromeSetupDialog } from "./ChromeDialogs";
import { ChromeConnection } from "./ChromeConnection";
import { chromeAvailable, setupLine, stepLabel } from "./chromeRows";
import { useChromeControl } from "./useChromeControl";
import { term, TEXT } from "../../../vocabulary";
/** The section. */
export function YourChromeSettings({
  browser = "Chrome",
}: {
  browser?: "Chrome" | "Edge" | "Firefox";
}): ReactElement | null {
  const chrome = useChromeControl(browser);
  const edge = browser !== "Chrome";
  const prefix = browser === "Firefox" ? "firefox" : edge ? "edge" : "chrome";
  // The switch turns on the same thing the vw:chrome skill names.
  const useMyChrome = term("builtinSkills", "vw:chrome").name;
  const [setupOpen, setSetupOpen] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);
  const view = chrome.view;
  const recovering = view?.setup.addon?.state === "connecting" && view.setup.addon.action === null;
  const setUp = view !== null && view.setup.state !== "notSetUp";
  const blocked = view?.setup.pairing === "Blocked by your organization";
  const busy = chrome.busy !== null || view === null;
  if (!chromeAvailable(view)) return null;
  return (
    <section aria-labelledby={`${prefix}-title`} data-setting={`${prefix}-control`}>
      <h3 id={`${prefix}-title`}>
        {browser === "Firefox" ? TEXT.firefoxName : edge ? TEXT.edgeName : TEXT.chromeName}
      </h3>
      {browser !== "Firefox" && view?.setup.addon !== undefined ? (
        <p className="key-line">
          {view.setup.addon.extensionVersion
            ? `Extension ${view.setup.addon.extensionVersion}`
            : "Extension not installed"}
        </p>
      ) : null}
      {edge ? (
        <>
          {view === null && chrome.readError === null ? (
            <p className="key-line" role="status">
              Reading…
            </p>
          ) : null}
          <SettingsRow data-setting={`${prefix}-control-status`}>
            <span>
              Connection
              <span className="d" role="status">
                {chrome.readError !== null && view === null
                  ? "State unavailable"
                  : browser === "Firefox"
                    ? view?.setup.pairing
                    : setupLine(view, browser)}
              </span>
            </span>
            {browser === "Firefox" ? (
              <Button
                data-testid="settings.firefox-window"
                shape="outlined"
                disabled={
                  busy || blocked || !view.setup.package || view.setup.pairing === "Not installed"
                }
                onClick={() => {
                  void chrome.run(
                    {
                      op: view?.setup.pairing === "Window open" ? "closeFirefox" : "openFirefox",
                    },
                    "window",
                  );
                }}
              >
                {view?.setup.pairing === "Window open" ? "Close" : "Open window"}
              </Button>
            ) : !view?.setup.package ? null : setUp ? (
              <Button
                data-testid="settings.edge-setup-open"
                shape="outlined"
                disabled={busy || blocked}
                onClick={() => {
                  if (view.setup.addon?.action === "setupAgain") {
                    void chrome.run({ op: "openStore" }, "openStore");
                    return;
                  }
                  setSetupOpen(true);
                  if (
                    view.setup.pairing !== "Paired" &&
                    view.setup.addon?.state !== "chromeClosed" &&
                    view.setup.addon?.state !== "turnedOffInChrome" &&
                    !recovering &&
                    !view.setup.storeReview
                  )
                    void chrome.run({ op: "openStore" }, "openStore");
                }}
              >
                {/* Turned off = set up already: the fix is in Edge itself, not the Store. */}
                {view.setup.addon?.action === "setupAgain"
                  ? stepLabel(view.setup.addon.action)
                  : recovering ||
                      view.setup.pairing === "Paired" ||
                      view.setup.addon?.state === "chromeClosed" ||
                      view.setup.addon?.state === "turnedOffInChrome"
                    ? "Details"
                    : view.setup.addon?.action === "openPairingPage"
                      ? stepLabel(view.setup.addon.action)
                      : view.setup.addon?.state === "notConnected" ||
                          view.setup.addon?.state === "extensionNotInstalled"
                        ? "Connect Edge"
                        : "Finish setting up…"}
              </Button>
            ) : (
              <Button
                data-testid="settings.edge-setup-open"
                shape="outlined"
                disabled={blocked}
                onClick={() => {
                  setSetupOpen(true);
                }}
              >
                Set up…
              </Button>
            )}
          </SettingsRow>
          {browser === "Edge" && recovering && view.setup.addon?.step ? (
            <p className="key-line">{view.setup.addon.step}</p>
          ) : null}
        </>
      ) : (
        <ChromeConnection chrome={chrome} />
      )}
      {setUp && !edge ? (
        // Shown from setup on, like Remove: before setup the stored switch is on by default and does nothing yet.
        <SettingsRow data-setting="chrome-control-enabled">
          <span>{useMyChrome}</span>
          <Switch
            data-testid="settings.chrome-enabled"
            aria-label={useMyChrome}
            aria-checked={view.enabled}
            disabled={busy}
            onClick={() => {
              void chrome.run({ op: "setEnabled", enabled: !view.enabled }, "setEnabled");
            }}
          />
        </SettingsRow>
      ) : null}
      {edge ? (
        <p
          className={chrome.status?.alert === true ? "key-problem" : "key-line"}
          role={chrome.status?.alert === true ? "alert" : "status"}
          aria-live="polite"
          data-chrome-status=""
        >
          {chrome.status?.text ?? ""}
        </p>
      ) : null}
      {setUp && !edge ? (
        <SettingsRow data-setting="chrome-control-remove">
          <span>Remove from Vibe Work</span>
          <Button
            data-testid="settings.chrome-remove-open"
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
      {browser === "Edge" ? (
        <ChromeSetupDialog
          open={setupOpen}
          browser={browser}
          chrome={chrome}
          onClose={() => {
            setSetupOpen(false);
          }}
        />
      ) : null}
      <ChromeRemoveDialog
        open={removeOpen}
        chrome={chrome}
        onClose={() => {
          setRemoveOpen(false);
        }}
      />
    </section>
  );
}
