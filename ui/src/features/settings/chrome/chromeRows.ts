/**
 * The words and order of Settings › Browser › Your Chrome: the three levels (each with its one contract line), the
 * status line of the setup, the Safety page's Chrome rows, and the Settings search entries. Level names and contract
 * lines come from `vocabulary.json` (group `chromeControl`).
 * Pure. Depends on: protocol/core types, vocabulary. Called by: YourChromeSettings, the catalog, the Safety rows.
 */
import type { ChromeControlView, ChromeGroup, ChromeLevel } from "../../../protocol/core";
import { TEXT, term } from "../../../vocabulary";
import type { SettingEntry } from "../catalog";

/** Package presence controls every Chrome surface; a broken shipped package still offers repair. */
export function chromeAvailable(view: ChromeControlView | null | undefined): boolean {
  return view != null && view.setup.state !== "absent";
}

/** The levels, least to most; each includes the ones before it. */
export const LEVELS: readonly ChromeLevel[] = ["browse", "fill", "act"];

/** A level's or group's name. */
export function chromeName(id: ChromeGroup | ChromeLevel): string {
  return term("chromeControl", id).name;
}

/** A level's one contract line. */
export function contract(level: ChromeLevel): string {
  return term("chromeControl", level).description;
}

/** The status row of the setup. */
export function setupLine(view: ChromeControlView | null): string {
  if (view === null) return "Reading…";
  const setup = view.setup;
  switch (setup.state) {
    case "absent":
      return setup.pairing;
    case "notSetUp":
      return setup.package ? "Not set up." : `Not set up. ${setup.pairing}`;
    case "waitingForExtension":
    case "extensionAdded":
      return `${setup.pairing}${setup.pairing.endsWith(".") ? "" : "."}${setup.fingerprint ? ` Compare with Chrome: ${setup.fingerprint}.` : ""}`;
  }
}

/** The Safety page's "Your Chrome" row: the level and its contract, or `null` while not set up. */
export function safetyLine(view: ChromeControlView | null): string | null {
  if (!chromeAvailable(view) || view === null || view.setup.state === "notSetUp") return null;
  return `${chromeName(view.level)}: ${contract(view.level)}`;
}

/** The Safety page's "Also asks first" row. */
export function askFirstLine(view: ChromeControlView | null): string | null {
  if (!chromeAvailable(view) || view === null || view.setup.state === "notSetUp") return null;
  return `${chromeName("bookmarks")} and ${chromeName("history").toLowerCase()}: Vibe asks every time.`;
}

/** The Settings search entries of Your Chrome (`catalog.ts`). */
export function chromeCatalog(): SettingEntry[] {
  return [
    {
      id: "chrome-control",
      category: "browser",
      label: "Your Chrome",
      description:
        "Let Vibe work in your own Chrome, in tabs you pick, at the level of trust you choose here.",
      keywords: "chrome extension your browser signed in tabs control permissions",
    },
    {
      id: "chrome-control-status",
      category: "browser",
      label: "Chrome Control",
      description: "Set up Chrome Control and add its extension from the Chrome Web Store.",
      keywords: "chrome control set up install extension store helper connected",
    },
    {
      id: "chrome-control-level",
      category: "browser",
      label: TEXT.chromeLevelTitle,
      description: TEXT.chromeLevelHelp,
      keywords:
        "chrome trust level basic browser control advanced full works on your behalf permissions sign in forms",
    },
  ];
}
