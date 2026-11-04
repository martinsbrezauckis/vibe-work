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
    view.setup.pairing === "Paired"
  );
}
export function setupLine(
  view: ChromeControlView | null,
  browser: "Chrome" | "Edge" = "Chrome",
): string {
  if (browser === "Edge") {
    if (view === null) return "Reading…";
    if (!view.setup.package && view.setup.pairing === TEXT.chromePackageMissing)
      return wording(TEXT.browserPackageMissing, { browser: TEXT.edgeName });
    if (!view.setup.package && view.setup.pairing === TEXT.chromePackageChanged)
      return wording(TEXT.browserPackageChanged, { browser: TEXT.edgeName });
    if (!view.setup.package || view.setup.pairing === "Blocked by your organization")
      return view.setup.pairing;
    if (view.setup.addon) return view.setup.addon.label;
    return view.setup.state === "notSetUp" ? "Not set up" : view.setup.pairing;
  }
  if (view === null) return "Reading…";
  if (view.setup.addon) return view.setup.addon.label;
  if (!view.setup.package) return view.setup.pairing;
  if (chromeUsable(view)) return "Connected";
  if (view.setup.pairing === "Paired" && !view.enabled) return "Off";
  if (view.setup.browserIssue === "missing") return "Google Chrome not found";
  if (view.setup.browserIssue === "sandboxed") return "Snap or Flatpak Chrome is unsupported";
  if (view.setup.browserIssue === "differentBrowser") return "Google Chrome required";
  if (view.setup.storeReview) return "Extension in review";
  if (view.setup.state === "notSetUp") return "Not connected";
  if (view.setup.pairing.includes("another Vibe Work"))
    return "Different Vibe Work · pair in Chrome";
  if (view.setup.pairing === "Waiting for you to pair in Chrome") return "Pair in Chrome";
  if (view.setup.pairing === "Connecting") return "Connecting";
  if (view.setup.installState === "restartChrome") return "Restart Chrome";
  if (view.setup.installState === "external") return "Enable in Chrome";
  if (view.setup.state === "waitingForExtension") return "Add to Chrome";
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

/** Edge has its own pairing state; the shared browser connection must also be enabled. */
export function edgeUsable(view: ChromeControlView | null | undefined): boolean {
  return (
    view?.edge !== undefined &&
    view.enabled &&
    view.edge.package &&
    view.edge.state === "extensionAdded" &&
    view.edge.pairing === "Paired"
  );
}

/** Firefox reports its own pairing and the same shared availability. */
export function firefoxUsable(view: ChromeControlView | null | undefined): boolean {
  return (
    view?.firefox !== undefined &&
    view.enabled &&
    view.firefox.package &&
    view.firefox.state === "extensionAdded" &&
    view.firefox.pairing === "Window open"
  );
}

/** The card's one button for the current step; null when the step is in Chrome or needs nothing. */
export function stepAction(view: ChromeControlView | null): ChromeAddonAction | null {
  if (view === null) return "connect";
  if (view.setup.addon) return view.setup.addon.action;
  return chromeUsable(view) ? null : "connect";
}
/** The button's words for a step. */
export function stepLabel(action: ChromeAddonAction): string {
  if (action === "openPairingPage") return "Open pairing page";
  if (action === "setupAgain") return "Set up again";
  if (action === "turnOn") return "Turn on";
  return "Connect Chrome";
}
/** The compare code belongs to the pairing step only. */
export function pairingStep(view: ChromeControlView | null): boolean {
  if (view === null) return false;
  const state = view.setup.addon?.state;
  if (state !== undefined) return state === "notPaired" || state === "otherOwner";
  return (
    view.setup.pairing === "Waiting for you to pair in Chrome" ||
    view.setup.pairing.includes("another Vibe Work")
  );
}
