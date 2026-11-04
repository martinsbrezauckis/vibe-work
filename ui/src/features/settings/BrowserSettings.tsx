/** Browser Settings category: groups the Vibe Work browser, Your Chrome, Your Edge, Your Firefox and settings shared by all browsers. Uses the sections' confirmed state and errors; called by SettingsPage. */
import { useCoreState } from "../shell/hooks";
import { useState } from "react";
import { BrowserShieldsSettings } from "./BrowserShieldsSettings";
import { BrowserPoolSettings } from "./BrowserPoolSettings";
import { BrowserDelegationSettings } from "./BrowserDelegationSettings";
import { YourChromeSettings } from "./chrome/YourChromeSettings";

/** Errors belong to the Browser page rather than to both section cards. */
export function BrowserSettings() {
  const firefox = useCoreState((s) => s.chromeControl?.firefox !== undefined);
  const edge = useCoreState((s) => s.chromeControl?.edge !== undefined);
  const [poolError, setPoolError] = useState<string | null>(null);
  const [shieldsError, setShieldsError] = useState<string | null>(null);
  const error = poolError ?? shieldsError;
  return (
    <>
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
      <section aria-labelledby="vibe-work-browser-title">
        <h3 id="vibe-work-browser-title">Vibe Work browser</h3>
        <BrowserPoolSettings onError={setPoolError} />
        <BrowserShieldsSettings onError={setShieldsError} />
      </section>
      <YourChromeSettings />
      {edge ? <YourChromeSettings browser="Edge" /> : null}
      {firefox ? <YourChromeSettings browser="Firefox" /> : null}
      <section aria-labelledby="all-browsers-title">
        <h3 id="all-browsers-title">All browsers</h3>
        <BrowserDelegationSettings />
      </section>
    </>
  );
}
