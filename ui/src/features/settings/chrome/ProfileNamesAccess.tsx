/**
 * Shown in a Chrome or Edge card only when the core reports that macOS refused reading that browser's profile folder
 * (`setup.profileNamesRefused`), so its profiles have numbered names: one line saying how to allow it and a button that
 * opens System Settings at Files & Folders. While it is shown, opening Browser settings and every return to the window
 * ask the core to read the folder again; once the person allowed it, the names appear and the line goes, without a
 * restart. Never shown on Windows or Linux (the core never reports the fact there). Used by `YourChromeSettings`.
 */
import { useEffect } from "react";
import { Button, SettingsRow } from "../../../components/Controls";
import { TEXT, wording } from "../../../vocabulary";
import { useApp } from "../../shell/AppContext";
import type { ChromeControlState } from "./useChromeControl";

/** One recheck at a time for every card: the core reads all refused folders in one go. */
let rechecking: Promise<unknown> | null = null;

export function ProfileNamesAccess({
  browser,
  chrome,
}: {
  browser: "Chrome" | "Edge";
  chrome: ChromeControlState;
}) {
  const { core } = useApp();
  const refused = chrome.view?.setup.profileNamesRefused === true;

  useEffect(() => {
    if (!refused || core === null) return;
    const recheck = () => {
      if (rechecking !== null || document.visibilityState === "hidden") return;
      rechecking = core
        .run({ type: "chromeControl", request: { op: "recheckProfileNames" } })
        .catch(() => undefined)
        .finally(() => {
          rechecking = null;
        });
    };
    recheck();
    window.addEventListener("focus", recheck);
    document.addEventListener("visibilitychange", recheck);
    return () => {
      window.removeEventListener("focus", recheck);
      document.removeEventListener("visibilitychange", recheck);
    };
  }, [refused, core]);

  if (!refused) return null;
  const text = wording(TEXT.browserProfileNamesAccess, {
    browser,
    app: browser === "Edge" ? "Microsoft Edge" : "Google Chrome",
  });
  return (
    <SettingsRow data-setting={`${browser.toLowerCase()}-profile-names-access`}>
      <span>{text}</span>
      <Button
        data-testid={`settings.${browser.toLowerCase()}-profile-names-settings`}
        shape="outlined"
        disabled={chrome.busy !== null}
        onClick={() => {
          void chrome.run({ op: "openProfileNameSettings" }, "openProfileNameSettings");
        }}
      >
        {TEXT.openSystemSettings}
      </Button>
    </SettingsRow>
  );
}
