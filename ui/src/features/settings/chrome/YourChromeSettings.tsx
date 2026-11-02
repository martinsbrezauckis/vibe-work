/** Browser availability and connection controls. Uses the core's confirmed state; called by BrowserSettings. */
import { type ReactElement, useState } from "react";
import { Button, SettingsRow, Switch } from "../../../components/Controls";
import { ChromeRemoveDialog, ChromeSetupDialog } from "./ChromeDialogs";
import { chromeAvailable, setupLine } from "./chromeRows";
import { useChromeControl } from "./useChromeControl";
import { term, TEXT } from "../../../vocabulary";
/** The section. */
export function YourChromeSettings(): ReactElement | null {
  const chrome = useChromeControl();
  // The switch turns on the same thing the vw:chrome skill names.
  const useMyChrome = term("builtinSkills", "vw:chrome").name;
  const [setupOpen, setSetupOpen] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);
  const view = chrome.view;
  const setUp = view !== null && view.setup.state !== "notSetUp";
  const busy = chrome.busy !== null || view === null;
  if (!chromeAvailable(view)) return null;
  return (
    <section aria-labelledby="chrome-title" data-setting="chrome-control">
      <h3 id="chrome-title">{TEXT.chromeName}</h3>
      {view === null && chrome.readError === null ? (
        <p className="key-line" role="status">
          Reading…
        </p>
      ) : null}
      <SettingsRow data-setting="chrome-control-status">
        <span>
          Connection
          <span className="d" role="status">
            {chrome.readError !== null && view === null ? "State unavailable" : setupLine(view)}
          </span>
        </span>
        {setUp ? (
          <Button
            data-testid="settings.chrome-setup-open"
            shape="outlined"
            onClick={() => {
              setSetupOpen(true);
            }}
          >
            {view.setup.pairing === "Paired" ? "Details" : "Finish setting up…"}
          </Button>
        ) : (
          <Button
            data-testid="settings.chrome-setup-open"
            shape="outlined"
            disabled={view === null || !view.setup.package}
            onClick={() => {
              setSetupOpen(true);
            }}
          >
            Set up…
          </Button>
        )}
      </SettingsRow>
      {setUp ? (
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
      <p
        className={chrome.status?.alert === true ? "key-problem" : "key-line"}
        role={chrome.status?.alert === true ? "alert" : "status"}
        aria-live="polite"
        data-chrome-status=""
      >
        {chrome.status?.text ?? ""}
      </p>
      {setUp ? (
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
      <ChromeSetupDialog
        open={setupOpen}
        chrome={chrome}
        onClose={() => {
          setSetupOpen(false);
        }}
      />
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
