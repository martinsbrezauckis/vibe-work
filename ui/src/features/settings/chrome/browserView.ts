/** Projects one browser's confirmed core facts into the shared connection control; used by Settings and composer. */
import type { ChromeControlView } from "../../../protocol/core";

export type SettingsBrowser = "Chrome" | "Edge" | "Safari" | "Firefox";

export function browserView(
  view: ChromeControlView | null | undefined,
  browser: SettingsBrowser,
): ChromeControlView | null {
  if (view == null) return null;
  if (browser === "Chrome") return view;
  if (browser === "Edge")
    return view.edge
      ? { ...view, setup: view.edge, enabled: view.edgeEnabled ?? view.enabled }
      : null;
  if (browser === "Firefox")
    return view.firefox
      ? { ...view, setup: view.firefox, enabled: view.firefoxEnabled ?? view.enabled }
      : null;
  const safari = view.safari;
  if (!safari) return null;
  const state = safari.addon.state;
  return {
    ...view,
    enabled: safari.enabled,
    setup: {
      state: state === "notSetUp" || state === "unavailable" ? "notSetUp" : "extensionAdded",
      package: safari.package,
      browsers: ["Safari"],
      profiles: [],
      storeUrl: "",
      storeReview: false,
      pairingState:
        state === "connected"
          ? "paired"
          : state === "otherOwner"
            ? "otherOwner"
            : state === "notPaired"
              ? "waiting"
              : "disconnected",
      pairing: safari.addon.label,
      fingerprint: safari.fingerprint,
      addon: safari.addon,
    },
  };
}
