/**
 * Settings › Browser: reads the browser pool and saves its live tab limit.
 * Depends on: AppContext and the typed browser command. Called by: SettingsPage.
 * Invariants: a failed save keeps the last confirmed selection; memory and cap come from the core
 * and refresh while the section is open.
 */
import { type ReactElement, useEffect, useState } from "react";

import { Selector } from "../../components/Selector";
import type { BrowserResult } from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import { useApp } from "../shell/AppContext";

type Pool = Extract<BrowserResult, { type: "enginePool" }>["pool"];
type Limit = "auto" | "1" | "2" | "3" | "4";
const OPTIONS: readonly Limit[] = ["auto", "1", "2", "3", "4"];

/** The live browser tab limit and memory estimate. */
export function BrowserPoolSettings({
  onError,
}: {
  onError: (error: string | null) => void;
}): ReactElement | null {
  const { core } = useApp();
  const [pool, setPool] = useState<Pool | null>(null);
  const [busy, setBusy] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const error = saveError ?? readError;
  useEffect(() => {
    onError(error);
  }, [error, onError]);

  useEffect(() => {
    if (core === null || busy) return;
    let live = true;
    let reading = false;
    const read = () => {
      if (reading) return;
      reading = true;
      void core
        .run({ type: "browser", request: { op: "enginePool" } })
        .then((result) => {
          if (!live) return;
          if (result.type !== "browser" || result.result.type !== "enginePool")
            throw new Error("Unexpected browser pool response.");
          setPool(result.result.pool);
          setReadError(null);
        })
        .catch((thrown: unknown) => {
          if (live) setReadError(toCallError(thrown).error.message);
        })
        .finally(() => {
          reading = false;
        });
    };
    read();
    const timer = window.setInterval(read, 5_000);
    const stop = core.listen((event) => {
      if (event.type === "browserChanged") read();
    });
    return () => {
      live = false;
      window.clearInterval(timer);
      stop();
    };
  }, [core, busy]);

  const change = (limit: Limit) => {
    if (core === null || busy || pool === null || limit === pool.configuredParallelSessions) return;
    setBusy(true);
    setSaveError(null);
    void core
      .run({ type: "browser", request: { op: "setEnginePool", configuredParallelSessions: limit } })
      .then((result) => {
        if (result.type !== "browser" || result.result.type !== "enginePool")
          throw new Error("Unexpected browser pool response.");
        setPool(result.result.pool);
      })
      .catch((thrown: unknown) => {
        setSaveError(toCallError(thrown).error.message);
      })
      .finally(() => {
        setBusy(false);
      });
  };

  if (core === null) return null;
  return (
    <section aria-labelledby="browser-pool-title">
      <h3 id="browser-pool-title">Browser</h3>
      <div className="check" data-setting="parallel-browser-sessions">
        <span>
          Live browser tabs
          <span className="d">
            How many browser tabs can stay live at once. Background tabs pause when the limit is
            reached. Auto adjusts to this computer&apos;s memory.
          </span>
          {pool === null && error === null && (
            <span className="d" role="status">
              Reading browser capacity…
            </span>
          )}
          {pool !== null && (
            <span className="d" role="status">
              Up to {pool.effectiveCap} live {pool.effectiveCap === 1 ? "tab" : "tabs"} at once.
              Memory:{" "}
              {pool.detectedRamGb === null ? "unavailable" : `${String(pool.detectedRamGb)} GB`};
              free memory:{" "}
              {pool.freeRamMb === null ? "unavailable" : `${String(pool.freeRamMb)} MB`}.
            </span>
          )}
        </span>
        <Selector
          testId="settings.browser-pool-settings-some"
          label="Live browser tabs"
          value={pool?.configuredParallelSessions ?? "auto"}
          disabled={pool === null || busy}
          options={OPTIONS.map((option) => ({
            value: option,
            label: option === "auto" ? "Auto (default)" : option,
          }))}
          onChange={(limit) => {
            if (OPTIONS.some((option) => option === limit)) change(limit as Limit);
          }}
        />
      </div>
    </section>
  );
}
