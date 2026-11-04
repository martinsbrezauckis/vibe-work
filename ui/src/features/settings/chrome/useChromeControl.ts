/** Confirmed Chrome connection state and settings commands. Used by the Browser section and dialogs. */
import { TEXT } from "../../../vocabulary";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

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
  connect: () => Promise<void>;
  run: (request: ChromeControlRequest, key: string) => Promise<CommandResult | null>;
}

/** Reads the view once (if the store has none yet) and runs changes. */
export function useChromeControl(
  browser: "Chrome" | "Edge" | "Firefox" = "Chrome",
): ChromeControlState {
  const { core } = useApp();
  const stored = useCoreState((s) => s.chromeControl);
  const view = useMemo(
    () =>
      browser === "Firefox"
        ? stored?.firefox === undefined
          ? null
          : { ...stored, setup: stored.firefox }
        : browser === "Edge"
          ? stored?.edge === undefined
            ? null
            : { ...stored, setup: stored.edge }
          : stored,
    [browser, stored],
  );
  const connecting = useRef(false);
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
        const selected: ChromeControlRequest =
          browser === "Firefox"
            ? request.op === "offerSetup"
              ? { op: "offerFirefoxSetup" }
              : request.op === "setup"
                ? { op: "setupFirefox", offerId: request.offerId }
                : request.op === "openStore"
                  ? { op: "openFirefoxStore" }
                  : request
            : browser === "Edge"
              ? request.op === "offerSetup"
                ? { op: "offerEdgeSetup" }
                : request.op === "setup"
                  ? { op: "setupEdge", offerId: request.offerId }
                  : request.op === "openStore"
                    ? { op: "openEdgeStore" }
                    : request
              : request;
        const result = await core.run({ type: "chromeControl", request: selected });
        if (request.op === "remove")
          setStatus({
            text:
              result.type === "chromeControl" && result.view.setup.extensionRestart === true
                ? "Removed. Restart Chrome to finish."
                : result.type === "chromeControl" && result.view.setup.extensionLeft === true
                  ? TEXT.chromeRemoved
                  : "Removed.",
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
    [core, browser],
  );

  const connect = useCallback(async () => {
    if (core === null || connecting.current) return;
    connecting.current = true;
    setBusy("connect");
    setStatus(null);
    try {
      const current = core.store.get().chromeControl;
      if (current?.setup.state === "notSetUp") {
        const offer = await core.run({ type: "chromeControl", request: { op: "offerSetup" } });
        if (offer.type !== "chromeControl" || offer.offer === null)
          throw new Error("Chrome setup unavailable");
        await core.run({
          type: "chromeControl",
          request: { op: "setup", offerId: offer.offer.offerId },
        });
      } else if (
        current?.setup.addon?.action === "turnOn" ||
        (current?.setup.addon === undefined && current?.setup.pairing === "Paired")
      ) {
        await core.run({ type: "chromeControl", request: { op: "setEnabled", enabled: true } });
      } else {
        await core.run({ type: "chromeControl", request: { op: "connect" } });
      }
    } catch (thrown) {
      setStatus({ text: toCallError(thrown).error.message, alert: true });
    } finally {
      connecting.current = false;
      setBusy(null);
    }
  }, [core]);

  const setupState = view?.setup.state;
  // Core owner notifications supply pairing; reads also discover extension installation and restart completion.
  useEffect(() => {
    if (
      core === null ||
      setupState === undefined ||
      setupState === "absent" ||
      setupState === "notSetUp"
    )
      return;
    const timer = setInterval(() => {
      void core.run({ type: "chromeControl", request: { op: "read" } }).catch(() => undefined);
    }, 3000);
    return () => {
      clearInterval(timer);
    };
  }, [core, setupState]);

  return { view, busy, status, readError, run, connect };
}
