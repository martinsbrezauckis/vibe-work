/**
 * Whether a page (Settings, the Projects overview, the Tools page) lies over the work view. The work view
 * stays mounted underneath so Home returns to it unchanged; window-level pieces inside it (the transcript's link
 * dialog and sources panel) must then not render, or the page's own copy would be shown twice.
 * Depends on: react. Called by: Content (provides), Transcript (reads).
 */
import { createContext, useContext } from "react";

export const CoveredByPage = createContext(false);

/** True while a page covers the work view. */
export function useCoveredByPage(): boolean {
  return useContext(CoveredByPage);
}
