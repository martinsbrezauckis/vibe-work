/** Core-owned keep-awake preference/status for Settings. Subscribes before reading; errors stay inline. */
import { useEffect, useState } from "react";
import type { KeepAwakeView } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";

/** Read, observe and save the app preference; a failed save keeps the confirmed value. */
export function useKeepAwake(core: CoreController | null) {
  const [view, setView] = useState<KeepAwakeView | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (core === null) return;
    let live = true;
    let revision = 0;
    const read = () => {
      const started = revision;
      void core
        .run({ type: "keepAwake", request: { op: "read" } })
        .then((result) => {
          if (live && revision === started && result.type === "keepAwake") {
            setView(result.view);
            setError(null);
          }
        })
        .catch((thrown: unknown) => {
          if (live) setError(toCallError(thrown).error.message);
        });
    };
    const stop = core.listen((event) => {
      if (event.type === "keepAwake") {
        revision++;
        setView(event.view);
      }
      if (event.type === "lagged") read();
    });
    read();
    return () => {
      live = false;
      stop();
    };
  }, [core]);

  const change = async (enabled: boolean) => {
    if (core === null || view === null || pending) return;
    setPending(true);
    setError(null);
    try {
      const result = await core.run({ type: "keepAwake", request: { op: "write", enabled } });
      if (result.type === "keepAwake") setView(result.view);
    } catch (thrown) {
      setError(toCallError(thrown).error.message);
    } finally {
      setPending(false);
    }
  };
  return { view, pending, error, change };
}
