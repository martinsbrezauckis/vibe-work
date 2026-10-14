/**
 * The new-chat reasoning preference, separate from each model's runtime default.
 * Called by composerStore; reads and writes only the app's own storage key.
 * Unsupported choices are retained and validated against reported levels before use.
 */
const KEY = "vw.defaultReasoning";

/** Reads a saved level; missing or unavailable storage means follow the model. */
export function loadDefaultReasoning(): string | null {
  try {
    const value = localStorage.getItem(KEY);
    return value !== null && value !== "" && value.length <= 128 ? value : null;
  } catch {
    return null;
  }
}

/** Saves an explicit pick; null follows the model. Storage failures leave the in-memory pick usable. */
export function saveDefaultReasoning(level: string | null): void {
  try {
    if (level === null) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, level);
  } catch {
    // The current window keeps the pick when storage is unavailable.
  }
}

/** Returns the saved level only when the selected model reports it as offered. */
export function offeredReasoning(level: string | null, levels: unknown): string | null {
  return level !== null && Array.isArray(levels) && levels.includes(level) ? level : null;
}
