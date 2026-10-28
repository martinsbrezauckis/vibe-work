/**
 * The app's two announcement regions (`state/announcer.ts`): polite and assertive, always in the page so the first
 * message is read too, outside the window's layout and never made inert by a modal dialog (`data-announcer-root`).
 * Depends on: the announcer store. Called by: Shell.
 * Invariants: render only; invisible (`.sr`); nothing else in the app adds an app-level live region.
 */
import type { ReactElement } from "react";
import { useStore } from "../../state/createStore";
import { announcerStore } from "../../state/announcer";

/** The regions. */
export function Announcer(): ReactElement {
  const polite = useStore(announcerStore, (s) => s.polite);
  const assertive = useStore(announcerStore, (s) => s.assertive);
  return (
    <div className="sr" data-announcer-root="">
      <p aria-live="polite" aria-atomic="true" data-announcer="polite">
        {polite}
      </p>
      <p aria-live="assertive" aria-atomic="true" data-announcer="assertive">
        {assertive}
      </p>
    </div>
  );
}
