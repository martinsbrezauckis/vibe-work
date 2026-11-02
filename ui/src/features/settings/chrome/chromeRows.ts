/** Browser connection labels and search entries. Used by Settings and its Debug API route. */
import type { ChromeControlView } from "../../../protocol/core";
import type { SettingEntry } from "../catalog";
import { term, TEXT } from "../../../vocabulary";
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
export function setupLine(view: ChromeControlView | null): string {
  if (view === null) return "Reading…";
  if (!view.setup.package) return view.setup.pairing;
  if (view.setup.state === "notSetUp") return "Not set up";
  return view.setup.pairing;
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
      label: term("builtinSkills", "vw:chrome").name,
      description: "",
      keywords: "chrome on off",
    },
  ];
}
