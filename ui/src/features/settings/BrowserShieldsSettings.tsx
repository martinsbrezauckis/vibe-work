/** Native browser ad-filter selector. Uses the typed core and confirmed values; rendered by BrowserSettings. */
import { useEffect, useState } from "react";
import { Selector } from "../../components/Selector";
import type { BrowserResult } from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import { useApp } from "../shell/AppContext";

type Settings = Extract<BrowserResult, { type: "shields" }>["settings"];
const OPTIONS = [
  { value: "off", label: "Off" },
  { value: "balanced", label: "Balanced" },
  { value: "strict", label: "Strict" },
] as const;

/** An unsuccessful download or save leaves the confirmed mode selected. */
export function BrowserShieldsSettings({ onError }: { onError: (error: string | null) => void }) {
  const { core } = useApp();
  const [settings, setSettings] = useState<Settings | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    onError(error);
  }, [error, onError]);
  useEffect(() => {
    if (core === null) return;
    let live = true;
    void core
      .run({ type: "browser", request: { op: "shields" } })
      .then((reply) => {
        if (live && reply.type === "browser" && reply.result.type === "shields")
          setSettings(reply.result.settings);
      })
      .catch((thrown: unknown) => {
        if (live) setError(toCallError(thrown).error.message);
      });
    return () => {
      live = false;
    };
  }, [core]);
  const change = (value: string) => {
    if (
      core === null ||
      settings === null ||
      busy ||
      value === settings.mode ||
      (value !== "off" && value !== "balanced" && value !== "strict")
    )
      return;
    setBusy(true);
    setError(null);
    void core
      .run({ type: "browser", request: { op: "setShields", mode: value } })
      .then((reply) => {
        if (reply.type !== "browser" || reply.result.type !== "shields")
          throw new Error("Ad blocking unavailable");
        setSettings(reply.result.settings);
      })
      .catch((thrown: unknown) => {
        setError(toCallError(thrown).error.message);
      })
      .finally(() => {
        setBusy(false);
      });
  };
  if (core === null) return null;
  return (
    <section aria-labelledby="ad-blocking-title">
      <h3 id="ad-blocking-title">Ads and trackers</h3>
      <div className="check" data-setting="browser-ad-blocking">
        <span>
          Ad blocking
          {busy && (
            <span className="d" role="status">
              Saving…
            </span>
          )}
          {settings !== null && !settings.supported && <span className="d">Unavailable</span>}
        </span>
        <Selector
          testId="settings.browser-ad-blocking"
          label="Ad blocking"
          value={settings?.mode ?? "balanced"}
          options={OPTIONS}
          disabled={settings === null || !settings.supported || busy}
          onChange={change}
        />
      </div>
    </section>
  );
}
