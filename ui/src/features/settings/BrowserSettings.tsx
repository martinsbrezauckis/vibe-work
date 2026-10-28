/** Browser Settings category: presents one page-level error above capacity, Your Chrome (Chrome Control) and delegation sections. Uses the sections' confirmed state and errors; called by SettingsPage. */
import { useState } from "react";
import { BrowserPoolSettings } from "./BrowserPoolSettings";
import { BrowserDelegationSettings } from "./BrowserDelegationSettings";
import { YourChromeSettings } from "./chrome/YourChromeSettings";

/** Errors belong to the Browser page rather than to both section cards. */
export function BrowserSettings() {
  const [poolError, setPoolError] = useState<string | null>(null);
  const [delegationError, setDelegationError] = useState<string | null>(null);
  const error = poolError ?? delegationError;
  return (
    <>
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
      <BrowserPoolSettings onError={setPoolError} />
      <YourChromeSettings />
      <BrowserDelegationSettings onError={setDelegationError} />
    </>
  );
}
