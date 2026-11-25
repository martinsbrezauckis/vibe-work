/** Browser availability and shared connection controls. Uses confirmed core state; called by BrowserSettings. */
import { type ReactElement, useState } from "react";
import { Button, SettingsRow, Switch } from "../../../components/Controls";
import { BrowserComposerSettings } from "../BrowserComposerSettings";
import { ChromeRemoveDialog } from "./ChromeDialogs";
import { FirefoxConnection } from "./FirefoxConnection";
import { BrowserProfiles } from "./BrowserProfiles";
import { ProfileNamesAccess } from "./ProfileNamesAccess";
import { ChromeConnection } from "./ChromeConnection";
import { chromeAvailable } from "./chromeRows";
import { ACT_REMOVE_ELLIPSIS } from "../actionLabels";
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
  // The extension line states only what the core checked. It looks in the browser's profiles only while the
  // browser is set up; before Connect and after Remove it has nowhere to look, so no "not installed" is claimed then,
  // except that after Remove it may report the extension still in a profile.
  const addon = firefox || view?.setup.browserIssue !== undefined ? undefined : view?.setup.addon;
  // While the system keyring blocks the helper, the browser's extension was not checked: the card names the keyring.
  const extensionLine =
    addon === undefined ||
    addon.state === "ownerKeyringUnavailable" ||
    (browser === "Safari" && !addon.extensionVersion)
      ? null
      : addon.extensionVersion
        ? `Extension ${addon.extensionVersion}`
        : view?.setup.installState === "store"
          ? "Extension version unknown"
          : view?.setup.extensionLeft === true || view?.setup.extensionRestart === true
            ? wording(TEXT.browserExtensionStillThere, { browser })
            : view?.setup.state === "notSetUp" || view?.setup.state === "absent"
              ? null
              : `Extension not found in ${browser}`;
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
      {extensionLine === null ? null : <p className="key-line">{extensionLine}</p>}
      {browser === "Safari" && view?.safari?.otherAppCopies?.length ? (
        <SettingsRow data-setting="safari-other-copies">
          <span>
            Other app copies
            {view.safari.otherAppCopies.map((path) => (
              <span className="d" key={path}>
                {path}
              </span>
            ))}
          </span>
        </SettingsRow>
      ) : null}
      {firefox ? (
        <FirefoxConnection chrome={chrome} />
      ) : (
        <ChromeConnection chrome={chrome} browser={browser} showSwitch />
      )}
      <BrowserProfiles browser={browser} chrome={chrome} />
      {browser === "Chrome" || browser === "Edge" ? (
        <ProfileNamesAccess browser={browser} chrome={chrome} />
      ) : null}
      <BrowserComposerSettings browser={browser} />
      {setUp || browser === "Safari" || firefox ? (
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
      {(setUp || firefox) && !removeRequired ? (
        <SettingsRow data-setting={`${prefix}-control-remove`}>
          <p>Remove from Vibe Work</p>
          <Button
            data-testid={`settings.${prefix}-remove-open`}
            aria-label={`Remove ${browserTitle(browser)} from Vibe Work`}
            shape="outlined"
            variant="danger"
            onClick={() => {
              setRemoveOpen(true);
            }}
          >
            {ACT_REMOVE_ELLIPSIS}
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

/** The card's title, for the whole-browser Remove button's own name (each card's button names its browser). */
function browserTitle(browser: string): string {
  if (browser === "Firefox") return TEXT.firefoxName;
  if (browser === "Safari") return TEXT.safariName;
  if (browser === "Edge") return TEXT.edgeName;
  return TEXT.chromeName;
}
