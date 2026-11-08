/** Browser connection labels and search entries. Used by Settings and its Debug API route. */
import type { ChromeAddonAction, ChromeControlView } from "../../../protocol/core";
import type { SettingEntry } from "../catalog";
import { term, TEXT, wording } from "../../../vocabulary";
export function chromeAvailable(view: ChromeControlView | null | undefined): boolean {
  return view != null && view.setup.state !== "absent";
}
/**
 * Your Chrome can be used now: the package is there, the extension was added and is paired, and the switch is on.
 * `enabled` alone is the stored switch, which is on until the person turns it off (also before setup), so the
 * composer's Chrome tab button asks for all four.
 */
export function chromeUsable(view: ChromeControlView | null | undefined): boolean {
  return (
    view != null &&
    view.enabled &&
    view.setup.package &&
    view.setup.state === "extensionAdded" &&
    view.setup.pairingState === "paired" &&
    view.setup.browserIssue === undefined &&
    (view.setup.addon === undefined || view.setup.addon.state === "connected")
  );
}
export function setupLine(
  view: ChromeControlView | null,
  browser: "Chrome" | "Edge" | "Safari" = "Chrome",
): string {
  if (view === null) return "Reading…";
  if (view.setup.addon) return view.setup.addon.label;
  if (browser === "Edge") {
    if (!view.setup.package && view.setup.pairing === TEXT.chromePackageMissing)
      return wording(TEXT.browserPackageMissing, { browser: TEXT.edgeName });
    if (!view.setup.package && view.setup.pairing === TEXT.chromePackageChanged)
      return wording(TEXT.browserPackageChanged, { browser: TEXT.edgeName });
    if (!view.setup.package || view.setup.pairingState === "blocked") return view.setup.pairing;
  }
  if (!view.setup.package) return view.setup.pairing;
  if (chromeUsable(view)) return "Connected";
  if (view.setup.pairingState === "paired" && !view.enabled) return "Off";
  const fullBrowser = browser === "Edge" ? "Microsoft Edge" : "Google Chrome";
  if (view.setup.browserIssue === "missing") return `${fullBrowser} is not installed`;
  if (view.setup.browserIssue === "sandboxed") return `Snap or Flatpak ${browser} is unsupported`;
  if (view.setup.browserIssue === "differentBrowser") return `${fullBrowser} required`;
  if (view.setup.storeReview) return "Extension in review";
  if (view.setup.state === "notSetUp") return "Not connected";
  if (view.setup.pairingState === "otherOwner") return `Different Vibe Work · pair in ${browser}`;
  if (view.setup.pairingState === "waiting") return `Pair in ${browser}`;
  if (view.setup.pairingState === "connecting") return "Connecting";
  if (view.setup.installState === "restartChrome") return `Restart ${browser}`;
  if (view.setup.installState === "external") return `Enable in ${browser}`;
  if (view.setup.state === "waitingForExtension") return `Add to ${browser}`;
  return "Connecting";
}
export function safetyLine(view: ChromeControlView | null): string | null {
  if (!chromeAvailable(view) || view === null || view.setup.state === "notSetUp") return null;
  return view.enabled ? "On" : "Off";
}
export function chromeCatalog(): SettingEntry[] {
  return [
    {
      id: "chrome-control",
      category: "browser",
      label: TEXT.chromeName,
      description: "",
      keywords: "chrome extension browser setup connection",
    },
    {
      id: "chrome-control-enabled",
      category: "browser",
      label: `${TEXT.chromeName} · ${term("builtinSkills", "vw:chrome").name}`,
      description: "",
      keywords: "chrome on off",
    },
  ];
}

/** Edge's pairing and its own availability switch must both be ready. */
export function edgeUsable(view: ChromeControlView | null | undefined): boolean {
  return (
    view?.edge !== undefined &&
    (view.edgeEnabled ?? view.enabled) &&
    view.edge.package &&
    view.edge.state === "extensionAdded" &&
    view.edge.pairingState === "paired" &&
    view.edge.browserIssue === undefined &&
    (view.edge.addon === undefined || view.edge.addon.state === "connected")
  );
}

/** Firefox reports its own pairing and the same shared availability. */
export function firefoxUsable(view: ChromeControlView | null | undefined): boolean {
  return (
    view?.firefox !== undefined &&
    view.enabled &&
    view.firefox.package &&
    view.firefox.state === "extensionAdded" &&
    view.firefox.pairingState === "windowOpen"
  );
}

/** The card's one button for the current step; null when the step is in Chrome or needs nothing. */
export function stepAction(view: ChromeControlView | null): ChromeAddonAction | null {
  if (view === null) return "connect";
  if (view.setup.addon) return view.setup.addon.action;
  return chromeUsable(view) ? null : "connect";
}
/** The button's words for a step. */
export function stepLabel(
  action: ChromeAddonAction,
  browser: "Chrome" | "Edge" | "Safari" = "Chrome",
): string {
  if (action === "remove") return "Remove…";
  if (action === "openPairingPage") return "Open pairing page";
  if (action === "setupAgain") return "Set up again";
  if (action === "turnOn") return "Turn on";
  return `Connect ${browser}`;
}
/** The compare code belongs to the pairing step only. */
export function pairingStep(view: ChromeControlView | null): boolean {
  if (view === null) return false;
  const state = view.setup.addon?.state;
  if (state !== undefined) return state === "notPaired" || state === "otherOwner";
  return view.setup.pairingState === "waiting" || view.setup.pairingState === "otherOwner";
}
