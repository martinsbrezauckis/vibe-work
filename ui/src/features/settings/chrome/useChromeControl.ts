/** Confirmed Chrome connection state and settings commands. Used by the Browser section and dialogs. */
import { TEXT } from "../../../vocabulary";
import { useCallback, useEffect, useState } from "react";

import type {
  ChromeControlRequest,
  ChromeControlView,
  CommandResult,
} from "../../../protocol/core";
import { toCallError } from "../../../protocol/client/types";
import { useApp } from "../../shell/AppContext";
import { useCoreState } from "../../shell/hooks";

/** The section's status line: what the last change did. */
interface ChromeStatus {
  text: string;
  alert: boolean;
}

/** What the section reads and does. */
export interface ChromeControlState {
  view: ChromeControlView | null;
  /** The request in flight (`setEnabled`, `setup`, …), or null. */
  busy: string | null;
  status: ChromeStatus | null;
  readError: string | null;
  /** Runs one request; resolves to its result, or null when it did not go through (the status says why). */
  run: (request: ChromeControlRequest, key: string) => Promise<CommandResult | null>;
}

/** Reads the view once (if the store has none yet) and runs changes. */
export function useChromeControl(): ChromeControlState {
  const { core } = useApp();
  const view = useCoreState((s) => s.chromeControl);
  const [busy, setBusy] = useState<string | null>(null);
  const [status, setStatus] = useState<ChromeStatus | null>(null);
  const [readError, setReadError] = useState<string | null>(null);

  useEffect(() => {
    if (core === null) return;
    let live = true;
    void core.run({ type: "chromeControl", request: { op: "read" } }).catch((thrown: unknown) => {
      if (live) setReadError(toCallError(thrown).error.message);
    });
    return () => {
      live = false;
    };
  }, [core]);

  const run = useCallback(
    async (request: ChromeControlRequest, key: string): Promise<CommandResult | null> => {
      if (core === null) return null;
      setBusy(key);
      setStatus(null);
      try {
        const result = await core.run({ type: "chromeControl", request });
        if (request.op === "remove")
          setStatus({
            text: TEXT.chromeRemoved,
            alert: false,
          });
        return result;
      } catch (thrown) {
        const error = toCallError(thrown).error;
        setStatus(
          error.code === "confirmationDeclined"
            ? { text: "Not changed.", alert: false }
            : { text: error.message, alert: true },
        );
        return null;
      } finally {
        setBusy(null);
      }
    },
    [core],
  );

  return { view, busy, status, readError, run };
}
