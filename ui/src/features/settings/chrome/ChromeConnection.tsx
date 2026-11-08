/** Shared live connection control for Tools and Browser settings. The hook owns commands; only core state proves connection. */
import { useId, useState, type ReactElement } from "react";
import { Button, SettingsRow } from "../../../components/Controls";
import { ChromeRemoveDialog } from "./ChromeDialogs";
import { TEXT } from "../../../vocabulary";
import { chromeAvailable, pairingStep, setupLine, stepAction, stepLabel } from "./chromeRows";
import { useChromeControl, type ChromeControlState } from "./useChromeControl";

/** One app click starts setup; the selected browser keeps its installation and pairing consent. */
export function ChromeConnection({
  chrome,
  browser = "Chrome",
  showSwitch = false,
}: {
  chrome: ChromeControlState;
  browser?: "Chrome" | "Edge" | "Safari";
  showSwitch?: boolean;
}): ReactElement {
  const [removeOpen, setRemoveOpen] = useState(false);
  const view = chrome.view;
  const action = stepAction(view);
  const needsPair = pairingStep(view);
  const step =
    browser === "Safari" && needsPair
      ? `Toolbar: ${TEXT.safariName} → Pair`
      : view?.setup.addon?.step;
  const prefix = browser.toLowerCase();
  return (
    <div data-testid={`${prefix}-connect`}>
      <SettingsRow data-setting={`${prefix}-control-status`}>
        <span>
          Connection
          <span className="d" role="status" aria-live="polite">
            {chrome.readError !== null && view === null
              ? "State unavailable"
              : setupLine(view, browser)}
          </span>
        </span>
        {action !== null && !(showSwitch && action === "turnOn") ? (
          <Button
            data-testid={`${prefix}.connect`}
            data-step={action}
            variant="primary"
            disabled={
              chrome.busy !== null ||
              view === null ||
              (action !== "remove" &&
                (!view.setup.package ||
                  view.setup.storeReview ||
                  (browser === "Edge" && view.setup.pairingState === "blocked") ||
                  view.setup.browserIssue !== undefined))
            }
            onClick={() => {
              if (action === "remove") setRemoveOpen(true);
              else void chrome.connect();
            }}
          >
            {stepLabel(action, browser)}
          </Button>
        ) : null}
      </SettingsRow>
      {step ? <p className="key-line">{step}</p> : null}
      <ChromeRemoveDialog
        open={removeOpen}
        chrome={chrome}
        browser={browser}
        onClose={() => {
          setRemoveOpen(false);
        }}
      />
      {needsPair && view?.setup.fingerprint ? (
        <p className="key-line" data-chrome-fingerprint="">
          Compare identity · <code>{view.setup.fingerprint}</code>
        </p>
      ) : null}
      {chrome.status?.text ? (
        <p
          className={chrome.status.alert ? "key-problem" : "key-line"}
          role={chrome.status.alert ? "alert" : "status"}
        >
          {chrome.status.text}
        </p>
      ) : null}
    </div>
  );
}

/** The same control, on the Tools landing page and beside installed plugins. */
export function ChromeCard(): ReactElement | null {
  const chrome = useChromeControl();
  const title = useId();
  if (!chromeAvailable(chrome.view)) return null;
  return (
    <section className="cgroup" aria-labelledby={title}>
      <h3 id={title}>{TEXT.chromeName}</h3>
      <div className="local-plugin">
        <ChromeConnection chrome={chrome} />
      </div>
    </section>
  );
}
