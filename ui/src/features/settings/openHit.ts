/**
 * Opens a Settings search hit as what it is: a setting or a link to Mistral's pages opens its
 * Settings category with the row lit; a Tools page item (connectors, MCP servers) opens the Tools page.
 * Depends on: AppContext types, catalog. Called by: SettingsNav, SettingsPage, the palette.
 */
import type { App } from "../shell/AppContext";
import { matchedAction } from "./actionLabels";
import type { SettingEntry } from "./catalog";

/** Opens `entry`'s Settings page at that row (or Tools when `opens === "connectors"`), carrying any matched action control to focus. */
export function openHit(app: App, entry: SettingEntry, query?: string): void {
  if (entry.opens === "connectors") {
    app.shell.actions.closeSettings();
    app.projects?.close();
    void app.connectors?.actions.open();
    return;
  }
  const action =
    query === undefined ? undefined : (matchedAction(entry.actions, query) ?? undefined);
  if (app.shell.store.get().settings.open)
    app.shell.actions.setSettingsCategory(entry.category, entry.id, action);
  else app.shell.actions.openSettings(entry.category, entry.id, action);
}
