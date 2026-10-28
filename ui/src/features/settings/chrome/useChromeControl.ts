/**
 * Settings › Browser › Your Chrome: the view the core last said (`chromeControl`, kept in the core store) and the
 * changes the section makes. A change that lets Vibe do more opens the core's native confirmation; Cancel there
 * comes back as `confirmationDeclined` and the section says "Not changed." (the level the core sends is the truth).
 * Depends on: the typed command client, the core store, vocabulary. Called by: YourChromeSettings, the dialogs.
 * Invariant: controls show only what the core confirmed; nothing is applied optimistically.
 */
import { useCallback, useEffect, useState } from "react";

import type {
  ChromeControlRequest,
  ChromeControlView,
  CommandResult,
} from "../../../protocol/core";
import { toCallError } from "../../../protocol/client/types";
import { TEXT, wording } from "../../../vocabulary";
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
  /** The request in flight (`setLevel`, `setRow:send`, …), or null. */
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
        if (request.op === "setLevel") setStatus({ text: TEXT.chromeSaved, alert: false });
        if (request.op === "remove")
          setStatus({ text: "Removed. Remove the extension in Chrome too.", alert: false });
        return result;
      } catch (thrown) {
        const error = toCallError(thrown).error;
        setStatus(
          error.code === "confirmationDeclined"
            ? { text: "Not changed.", alert: false }
            : request.op === "setLevel"
              ? { text: wording(TEXT.chromeSaveFailed, { reason: error.message }), alert: true }
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
