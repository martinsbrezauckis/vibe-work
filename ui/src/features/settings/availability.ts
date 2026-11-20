/**
 * Which Settings categories this window offers: Updates only when the build has an update channel (a
 * package-managed or development build has none, and its section would be empty) or carries one that was refused
 * (the section then says why update checks are off). Every other category is always there.
 * Depends on: react, AppContext. Called by: SettingsNav, SettingsPage, settingsRoutes.
 */
import { useSyncExternalStore } from "react";

import type { App } from "../shell/AppContext";
import { useApp } from "../shell/AppContext";
import { SETTINGS_CATEGORIES, type SettingsCategory } from "./catalog";

/** The categories when Updates is or is not shown. */
function categories(updates: boolean): readonly SettingsCategory[] {
  return updates ? SETTINGS_CATEGORIES : SETTINGS_CATEGORIES.filter((c) => c.id !== "updates");
}

/** Whether Settings shows Updates: a channel, or a refused one to explain. */
function showsUpdates(
  state: { available: boolean; offReason: string | null } | undefined,
): boolean {
  return state !== undefined && (state.available || state.offReason !== null);
}

/** The categories `app` offers, in column order. */
export function offeredCategories(app: App): readonly SettingsCategory[] {
  return categories(showsUpdates(app.updates?.store.get()));
}

/** The same, as a hook (re-renders when the update channel's availability changes). */
export function useOfferedCategories(): readonly SettingsCategory[] {
  const store = useApp().updates?.store;
  const updates = useSyncExternalStore(
    (listener) => (store === undefined ? () => undefined : store.subscribe(listener)),
    () => showsUpdates(store?.get()),
  );
  return categories(updates);
}
