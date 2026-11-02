/** Read-only access and browser-availability rows for Safety settings. */
import { TEXT, term } from "../../../vocabulary";
import type { App } from "../../shell/AppContext";
import { allowedFor } from "./store";

/** The level rows, in page order. */
export type LevelRowId = "access" | "chrome";

/** What a level row shows. */
export interface LevelRow {
  id: LevelRowId;
  label: string;
  value: string;
  link: string;
}

/** The Chrome Control rows' words while it is not set up. */
const CHROME_NOT_SET_UP = TEXT.chromeNotSetUp;

/** "Ask: asks before tool use." for an access mode id; `null` = Vibe's own default. */
function accessLine(mode: string | null): string {
  if (mode === null) return `${TEXT.accessDefaultFallback}.`;
  const t = term("accessModes", mode);
  const description = t.description === "" ? TEXT.customAccessDescription : t.description;
  return `${t.name}: ${description.charAt(0).toLowerCase()}${description.slice(1)}`;
}

/** The access row's value from what the Vibe settings read gave. */
export function accessValue(input: {
  running: boolean;
  via: string | null;
  mode: string | null | undefined;
  error: string | null;
}): string {
  if (!input.running) return "Vibe is not running, so this cannot be read now.";
  if (input.via === null) return "Needs a session.";
  if (input.error !== null) return input.error;
  if (input.mode === undefined) return "Reading…";
  return accessLine(input.mode);
}

/** The rows, from the values read (`chrome`: `null` while Chrome Control is not set up). */
export function levelRows(
  access: string,
  chrome: string | null = null,
  chromeAvailable = true,
): LevelRow[] {
  const link = chrome === null ? "Set up in Browser" : "Change in Browser";
  return [
    {
      id: "access",
      label: "New sessions start in",
      value: access,
      link: "Change in Models and agent",
    },
    ...(chromeAvailable
      ? [
          {
            id: "chrome" as const,
            label: TEXT.chromeName,
            value: chrome ?? CHROME_NOT_SET_UP,
            link,
          },
        ]
      : []),
  ];
}

/** A row's link: opens the category (and the row, lit) where that level is set. */
export function openLevel(app: App, row: LevelRowId): void {
  const allowed = allowedFor(app);
  if (row === "access") allowed.showCategory("models", "default-access-mode");
  else allowed.showCategory("browser", "chrome-control-enabled");
}
