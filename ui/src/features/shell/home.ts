/**
 * Home: the first rail item, "back to my work". It leaves whatever page is on top (Settings, the
 * Projects overview, the Tools page, the Libraries page) and shows the work view exactly as it was: the selected session (never a new
 * one), its scroll, draft, side-panel tab and list folds, all of which stay mounted underneath those pages. With no
 * session selected it shows the start view. Esc on the Settings page does the same.
 * Depends on: AppContext types. Called by: Rail, SettingsPage (Esc), the `ui/rail` route, the palette's action.
 * Invariants: Home never changes the selection, the list's visibility or the panel; it only closes the pages above.
 */
import { summarize } from "../../state/sessionSummary";
import type { App } from "./AppContext";

/** What Home would show now (for the Debug API and tests). */
export function homeTarget(app: App): { sessionId: string | null; title: string | null } {
  const selected = app.shell.store.get().selected;
  const entry = selected === null ? undefined : app.core?.store.get().sessions[selected];
  const title = entry === undefined ? null : summarize(entry).title;
  return { sessionId: selected, title };
}

/** Whether a page lies over the work view (Home has something to do). */
export function awayFromHome(app: App): boolean {
  return (
    app.shell.store.get().settings.open ||
    (app.projects?.store.get().open ?? false) ||
    (app.connectors?.store.get().open ?? false) ||
    (app.libraries?.store.get().open ?? false)
  );
}

/** Leaves Settings, the Projects overview and the Tools page; the work view underneath shows again unchanged. */
export function goHome(app: App): void {
  app.shell.actions.closeSettings();
  app.projects?.close();
  app.connectors?.actions.close();
  app.libraries?.actions.close();
  if (app.shell.store.get().drawer) app.shell.actions.setDrawer(false);
}
