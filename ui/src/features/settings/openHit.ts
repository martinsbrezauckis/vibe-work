/**
 * Opens a Settings search hit as what it is: a setting or a link to Mistral's pages opens its
 * Settings category with the row lit; a Tools page item (connectors, MCP servers) opens the Tools page.
 * Depends on: AppContext types, catalog. Called by: SettingsNav, SettingsPage, the palette.
 */
import type { App } from "../shell/AppContext";
import type { SettingEntry } from "./catalog";

/** Opens `entry`. */
export function openHit(app: App, entry: SettingEntry): void {
  if (entry.opens === "connectors") {
    app.shell.actions.closeSettings();
    app.projects?.close();
    void app.connectors?.actions.open();
    return;
  }
  if (app.shell.store.get().settings.open)
    app.shell.actions.setSettingsCategory(entry.category, entry.id);
  else app.shell.actions.openSettings(entry.category, entry.id);
}
