/** Confirmed Chrome connection state and settings commands. Used by the Browser section and dialogs. */
import { browserView, type SettingsBrowser } from "./browserView";
import { chromeUsable, firefoxUsable } from "./chromeRows";
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

/** The section's status line: what the last change did, or guidance after a step (`hint`, never an alert). */
interface ChromeStatus {
  text: string;
  alert: boolean;
  /**
   * `connectError`: a Connect step failed (store or pairing page not opened, setup refused); it is moot once the connection
   * it was about exists. `enableError`: turning the browser on failed; it is moot once the browser is usable.
   */
  kind?: "removed" | "hint" | "connectError" | "enableError";
  /** After "Removed.": the core's note that the removal could not be saved (shown as an alert). */
  note?: string;
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
export function useChromeControl(browser: SettingsBrowser = "Chrome"): ChromeControlState {
  const { core } = useApp();
  const stored = useCoreState((s) => s.chromeControl);
  const view = useMemo(() => browserView(stored, browser), [browser, stored]);
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
          browser === "Safari"
            ? request.op === "setEnabled"
              ? { op: "setSafariEnabled", enabled: request.enabled }
              : request.op === "remove"
                ? { op: "removeSafari" }
                : request
            : browser === "Firefox"
              ? request.op === "setEnabled"
                ? { op: "setFirefoxEnabled", enabled: request.enabled }
                : request.op === "remove"
                  ? { op: "removeFirefox" }
                  : request.op === "offerSetup"
                    ? { op: "offerFirefoxSetup" }
                    : request.op === "setup"
                      ? { op: "setupFirefox", offerId: request.offerId }
                      : request.op === "openStore"
                        ? { op: "openFirefoxStore" }
                        : request
              : browser === "Edge"
                ? request.op === "remove"
                  ? { op: "removeEdge" }
                  : request.op === "setEnabled"
                    ? { op: "setEdgeEnabled", enabled: request.enabled }
                    : request.op === "offerSetup"
                      ? { op: "offerEdgeSetup" }
                      : request.op === "setup"
                        ? { op: "setupEdge", offerId: request.offerId }
                        : request.op === "openStore"
                          ? { op: "openEdgeStore" }
                          : request
                : request;
        const result = await core.run({ type: "chromeControl", request: selected });
        if (request.op === "remove") {
          const note = result.type === "chromeControl" ? result.hint : undefined;
          setStatus(
            note
              ? { kind: "removed", text: "Removed.", alert: true, note }
              : { kind: "removed", text: "Removed.", alert: false },
          );
        }
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
    // The step a failure belongs to, for the kind of its status line.
    let failure: "connectError" | "enableError" = "connectError";
    try {
      const current = core.store.get().chromeControl;
      if (browser === "Safari") {
        if (current?.safari?.enabled === false) {
          failure = "enableError";
          await core.run({
            type: "chromeControl",
            request: { op: "setSafariEnabled", enabled: true },
          });
          failure = "connectError";
        }
        if (current?.safari?.addon.action !== "turnOn") {
          const opened = await core.run({
            type: "chromeControl",
            request: { op: "openSafariSetup" },
          });
          if (opened.type === "chromeControl" && opened.hint)
            setStatus({ kind: "hint", text: opened.hint, alert: false });
        }
        return;
      }
      if (browser === "Firefox") {
        const state = current?.firefoxSharing?.state;
        if (state === "switchedOff") {
          failure = "enableError";
          await core.run({
            type: "chromeControl",
            request: { op: "setFirefoxEnabled", enabled: true },
          });
        } else if (state === "notSetUp" || state === "helperSetupRequired") {
          const offer = await core.run({
            type: "chromeControl",
            request: { op: "offerFirefoxSetup" },
          });
          if (offer.type !== "chromeControl" || offer.offer === null)
            throw new Error("Firefox setup unavailable");
          await core.run({
            type: "chromeControl",
            request: { op: "setupFirefox", offerId: offer.offer.offerId },
          });
        } else {
          await core.run({ type: "chromeControl", request: { op: "openFirefoxStore" } });
        }
        return;
      }
      const setup = browser === "Edge" ? current?.edge : current?.setup;
      if (setup?.state === "notSetUp") {
        const offer = await core.run({
          type: "chromeControl",
          request: { op: browser === "Edge" ? "offerEdgeSetup" : "offerSetup" },
        });
        if (offer.type !== "chromeControl" || offer.offer === null)
          throw new Error(`${browser} setup unavailable`);
        const done = await core.run({
          type: "chromeControl",
          request: { op: browser === "Edge" ? "setupEdge" : "setup", offerId: offer.offer.offerId },
        });
        // Connect replaced an old registration that pointed to a missing file: said once, and kept after pairing.
        if (done.type === "chromeControl" && done.hint)
          setStatus({ text: done.hint, alert: false });
      } else if (
        setup?.addon?.action === "turnOn" ||
        (setup?.addon === undefined && setup?.pairingState === "paired")
      ) {
        failure = "enableError";
        await core.run({
          type: "chromeControl",
          request: { op: browser === "Edge" ? "setEdgeEnabled" : "setEnabled", enabled: true },
        });
      } else {
        const asked = await core.run({
          type: "chromeControl",
          request: { op: browser === "Edge" ? "openEdgeStore" : "connect" },
        });
        // A profile that waits after "Not now" is asked again on its next reconnect; the hint says when.
        if (asked.type === "chromeControl" && asked.hint)
          setStatus({
            ...(asked.hint.startsWith("Connected.") ? {} : { kind: "hint" as const }),
            text: asked.hint,
            alert: false,
          });
      }
    } catch (thrown) {
      setStatus({ kind: failure, text: toCallError(thrown).error.message, alert: true });
    } finally {
      connecting.current = false;
      setBusy(null);
    }
  }, [core, browser]);

  const setupState = view?.setup.state;
  const pendingRemoval =
    status?.kind === "removed" &&
    (view?.setup.extensionRestart === true || view?.setup.extensionLeft === true);
  // Core owner notifications supply pairing; reads also discover extension installation and restart completion.
  useEffect(() => {
    if (
      core === null ||
      setupState === undefined ||
      setupState === "absent" ||
      (setupState === "notSetUp" && !pendingRemoval)
    )
      return;
    return core.watchBrowserSetup();
  }, [core, setupState, pendingRemoval]);

  // Pairing guidance is moot once the browser is connected; it is dropped, not only hidden, so a later
  // disconnect does not bring it back. (State adjusted during render: React re-renders before committing.)
  if (status?.kind === "hint" && view?.setup.pairingState === "paired") setStatus(null);
  // A failed Connect or Turn on is dropped the same way once what it was about is true, so the card cannot say
  // "Connected" under a red banner. A failure of another change (`run`) has no kind and stays until the next action.
  if (staleFailure(status, view, browser)) setStatus(null);
  const liveStatus =
    status?.kind === "removed"
      ? {
          ...status,
          text: [
            view?.setup.extensionRestart === true
              ? "Removed. Restart Chrome to finish."
              : view?.setup.extensionLeft === true
                ? TEXT.chromeRemoved
                : "Removed.",
            status.note,
          ]
            .filter(Boolean)
            .join(" "),
        }
      : status;
  return { view, busy, status: liveStatus, readError, run, connect };
}

/** Whether a connection failure's subject has since come true: paired (and live), or usable. */
function staleFailure(
  status: ChromeStatus | null,
  view: ChromeControlView | null,
  browser: SettingsBrowser,
): boolean {
  if (view === null) return false;
  if (status?.kind === "enableError")
    return browser === "Firefox" ? firefoxUsable(view) : chromeUsable(view);
  if (status?.kind !== "connectError") return false;
  if (browser === "Firefox") return view.firefoxSharing?.state === "connected";
  const addon = view.setup.addon;
  return (
    view.setup.pairingState === "paired" && (addon === undefined || addon.state === "connected")
  );
}
