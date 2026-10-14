/**
 * What survives a restart: the theme, the dragged panel widths and display preferences in
 * `localStorage`. Storage can
 * be missing, full or blocked (private windows, WebView policies), so every access is wrapped and
 * the app renders correctly without it.
 * Depends on: shellState types. Called by: shellStore.
 * Invariants: reads validate what they get (a stale or tampered value falls back to the default);
 * writes never throw.
 */
import { TAB_KINDS, type Theme, type ShellState } from "./shellState";

const THEME_KEY = "vw.theme";
const WIDTHS_KEY = "vw.panelWidths";
const USAGE_KEY = "vw.statusUsage";
const THINKING_KEY = "vw.showThinkingExpanded";

/** The stored values, validated. */
export interface Persisted {
  theme: Theme | null;
  widths: ShellState["widths"];
  /** Show the context readout in the status bar (default on). */
  showUsage: boolean;
  /** Default disclosure of transcript thinking blocks (default on). */
  showThinkingExpanded: boolean;
}

/** Reads the stored theme and widths. */
export function loadPersisted(): Persisted {
  let theme: Theme | null = null;
  let widths: ShellState["widths"] = {};
  let showUsage = true;
  let showThinkingExpanded = true;
  try {
    showUsage = localStorage.getItem(USAGE_KEY) !== "off";
    showThinkingExpanded = localStorage.getItem(THINKING_KEY) !== "off";
    const stored = localStorage.getItem(THEME_KEY);
    if (stored === "dark" || stored === "light") theme = stored;
    const raw = localStorage.getItem(WIDTHS_KEY);
    if (raw !== null) {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed === "object" && parsed !== null) {
        const clean: Partial<Record<(typeof TAB_KINDS)[number], number>> = {};
        for (const kind of TAB_KINDS) {
          const value = (parsed as Record<string, unknown>)[kind];
          if (typeof value === "number" && Number.isFinite(value) && value > 0) clean[kind] = value;
        }
        widths = clean;
      }
    }
  } catch {
    // Storage unavailable or corrupt: defaults.
  }
  return { theme, widths, showUsage, showThinkingExpanded };
}

/** Stores the theme and widths; failures are ignored. */
export function savePersisted(
  state: Pick<ShellState, "theme" | "widths" | "showUsage" | "showThinkingExpanded">,
): void {
  try {
    localStorage.setItem(THEME_KEY, state.theme);
    localStorage.setItem(WIDTHS_KEY, JSON.stringify(state.widths));
    localStorage.setItem(USAGE_KEY, state.showUsage ? "on" : "off");
    localStorage.setItem(THINKING_KEY, state.showThinkingExpanded ? "on" : "off");
  } catch {
    // Not persisted; the app keeps working.
  }
}

/** The theme to start with: the stored choice, else dark (the product default, tokens.css). */
export function startingTheme(stored: Theme | null): Theme {
  return stored ?? "dark";
}
