/** Retains one notice for each newly discovered plugin folder, including while Tools is closed.
 * Depends on typed plugin views and toasts. Called by the marketplace controller; never runs plugin code.
 */
import type { LocalPluginView } from "../protocol/core";
import type { Toasts } from "./toastStore";
import { displayPluginName } from "./pluginText";

const KEY = "vw.plugins.off.seen";

/** The seen set is stored locally when the app persists settings. */
export function createPluginNotices(toasts: Toasts, persist: boolean) {
  const seen = new Set<string>();
  if (persist) {
    try {
      const saved: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
      if (Array.isArray(saved))
        for (const path of saved) if (typeof path === "string") seen.add(path);
    } catch {
      /* Unreadable local settings start with an empty seen set. */
    }
  }
  return (plugins: readonly LocalPluginView[]): void => {
    let changed = false;
    for (const plugin of plugins) {
      const identity = `${plugin.path}\0${plugin.name}`;
      if (
        !plugin.off ||
        plugin.scope !== "global" ||
        !plugin.managed ||
        plugin.path === "" ||
        seen.has(identity)
      )
        continue;
      seen.add(identity);
      changed = true;
      const name = displayPluginName(plugin.name);
      toasts.push({
        kind: "warn",
        bellKind: "security",
        text: `Vibe Work found the plugin “${name}” in its plugins folder and left it off. Its MCP servers could run code in your chats. Turn it on in Tools if you know where it came from.`,
        plugin: { name, path: plugin.path, managed: plugin.managed },
      });
    }
    if (changed && persist) {
      try {
        localStorage.setItem(KEY, JSON.stringify([...seen]));
      } catch {
        /* The app run still keeps the seen set. */
      }
    }
  };
}
