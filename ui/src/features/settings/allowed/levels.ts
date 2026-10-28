/**
 * Settings › Safety, "How much Vibe may do without asking": read-only rows for each level, each
 * with a link to the one place the level is set (placement.md: link, never a second control). The words of each row
 * and the links' targets live here; AllowedLevels renders them.
 * - New sessions start in: Vibe's default access mode, read the way Models and agent reads it (through a session).
 * - Your Chrome / Also asks first: Chrome Control's level (or Custom) and its "Also asks first" rows, from the core's
 *   `chromeControl` view; "Chrome Control is not set up." with "Set up in Browser" until it is.
 * - Close your tabs: the Browser category's tab-closing permission (`browser permissions`, `closeUserTabs`).
 * Depends on: react, protocol/core, protocol/client, vocabulary, AppContext, allowed/store. Called by:
 * AllowedLevels, allowed/routes, tests.
 */
import { useEffect, useState } from "react";

import type { BrowserPermissions } from "../../../protocol/core";
import { toCallError } from "../../../protocol/client/types";
import { TEXT, term } from "../../../vocabulary";
import type { App } from "../../shell/AppContext";
import { useApp } from "../../shell/AppContext";
import { allowedFor } from "./store";

/** The level rows, in page order. */
export type LevelRowId = "access" | "chrome" | "askFirst" | "closeTabs";

/** What a level row shows. */
export interface LevelRow {
  id: LevelRowId;
  label: string;
  value: string;
  link: string;
}

/** The Chrome Control rows' words while it is not set up. */
const CHROME_NOT_SET_UP = "Chrome Control is not set up.";

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

/** The Close your tabs row's value. */
export function closeTabsValue(
  permissions: BrowserPermissions | null,
  error: string | null,
): string {
  if (error !== null) return `Could not read it: ${error}`;
  if (permissions === null) return "Reading…";
  return permissions.closeUserTabs
    ? "On: Vibe may close a tab you opened without asking."
    : "Off: Vibe asks before it closes a tab you opened.";
}

/** The rows, from the values read (`chrome` / `askFirst`: `null` while Chrome Control is not set up). */
export function levelRows(
  access: string,
  closeTabs: string,
  chrome: string | null = null,
  askFirst: string | null = null,
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
          { id: "chrome" as const, label: "Your Chrome", value: chrome ?? CHROME_NOT_SET_UP, link },
          {
            id: "askFirst" as const,
            label: "Also asks first",
            value: askFirst ?? CHROME_NOT_SET_UP,
            link,
          },
        ]
      : []),
    { id: "closeTabs", label: "Close your tabs", value: closeTabs, link: "Change in Browser" },
  ];
}

/** A row's link: opens the category (and the row, lit) where that level is set. */
export function openLevel(app: App, row: LevelRowId): void {
  const allowed = allowedFor(app);
  if (row === "access") allowed.showCategory("models", "default-access-mode");
  else if (row === "closeTabs") allowed.showCategory("browser", "browser-close-user-tabs");
  else if (row === "chrome") allowed.showCategory("browser", "chrome-control-level");
  else allowed.showCategory("browser", "chrome-control-bookmarks");
}

/** Reads the browser permissions once (as Settings › Browser does). */
export function useBrowserPermissions(): {
  permissions: BrowserPermissions | null;
  error: string | null;
} {
  const { core } = useApp();
  const [permissions, setPermissions] = useState<BrowserPermissions | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (core === null) return;
    let live = true;
    void core
      .run({ type: "browser", request: { op: "permissions" } })
      .then((response) => {
        if (!live) return;
        if (response.type === "browser" && response.result.type === "permissions")
          setPermissions(response.result.permissions);
        else setError("Unexpected browser permissions response.");
      })
      .catch((thrown: unknown) => {
        if (live) setError(toCallError(thrown).error.message);
      });
    return () => {
      live = false;
    };
  }, [core]);
  return { permissions, error };
}
