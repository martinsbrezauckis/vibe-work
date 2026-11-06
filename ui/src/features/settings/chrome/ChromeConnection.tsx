/** Shared live connection control for Tools and Browser settings. The hook owns commands; only core state proves connection. */
import { useId, useState, type ReactElement } from "react";
import { Button, SettingsRow } from "../../../components/Controls";
import { ChromeRemoveDialog } from "./ChromeDialogs";
import { TEXT } from "../../../vocabulary";
import { chromeAvailable, pairingStep, setupLine, stepAction, stepLabel } from "./chromeRows";
import { useChromeControl, type ChromeControlState } from "./useChromeControl";

/** One app click starts setup; Chrome keeps its own installation and pairing consent. */
export function ChromeConnection({
  chrome,
  browser = "Chrome",
}: {
  chrome: ChromeControlState;
  browser?: "Chrome" | "Edge";
}): ReactElement {
  const [removeOpen, setRemoveOpen] = useState(false);
  const view = chrome.view;
  const action = stepAction(view);
  const needsPair = pairingStep(view);
  return (
    <div data-testid="chrome-connect">
      <SettingsRow data-setting="chrome-control-status">
        <span>
          Connection
          <span className="d" role="status" aria-live="polite">
            {chrome.readError !== null && view === null
              ? "State unavailable"
              : setupLine(view, browser)}
          </span>
        </span>
        {action !== null ? (
          <Button
            data-testid="chrome.connect"
            data-step={action}
            variant="primary"
            disabled={
              chrome.busy !== null ||
              view === null ||
              (action !== "remove" &&
                (!view.setup.package ||
                  view.setup.storeReview ||
                  view.setup.browserIssue !== undefined))
            }
            onClick={() => {
              if (action === "remove") setRemoveOpen(true);
              else void chrome.connect();
            }}
          >
            {stepLabel(action)}
          </Button>
        ) : null}
      </SettingsRow>
      {view?.setup.addon?.step ? <p className="key-line">{view.setup.addon.step}</p> : null}
      <ChromeRemoveDialog
        open={removeOpen}
        chrome={chrome}
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
