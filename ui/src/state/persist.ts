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
const FILES_KEY = "vw.openFilesIn";
const THINKING_KEY = "vw.showThinkingExpanded";
const MARKS_KEY = "vw.markVibeLines";
const SAVE_FIRST_KEY = "vw.saveBeforeSend";
const OPEN_AS_KEY = "vw.openHtmlMarkdownAs";

/** The stored values, validated. */
export interface Persisted {
  theme: Theme | null;
  openFilesIn: ShellState["openFilesIn"];
  widths: ShellState["widths"];
  /** Show the context readout in the status bar (default on). */
  showUsage: boolean;
  /** Default disclosure of transcript thinking blocks (default on). */
  showThinkingExpanded: boolean;
  /** Mark the lines Vibe changes in open files (default on). */
  markVibeLines: boolean;
  /** Save edited open files before a message is sent (default on). */
  saveBeforeSend: boolean;
  /** How HTML and Markdown files open (default preview). */
  openHtmlMarkdownAs: ShellState["openHtmlMarkdownAs"];
}

/** Reads the stored theme and widths. */
export function loadPersisted(): Persisted {
  let theme: Theme | null = null;
  let widths: ShellState["widths"] = {};
  let openFilesIn: ShellState["openFilesIn"] = "tabs";
  let showUsage = true;
  let showThinkingExpanded = true;
  let markVibeLines = true;
  let saveBeforeSend = true;
  let openHtmlMarkdownAs: ShellState["openHtmlMarkdownAs"] = "preview";
  try {
    const openAs = localStorage.getItem(OPEN_AS_KEY);
    if (openAs === "code" || openAs === "side") openHtmlMarkdownAs = openAs;
    markVibeLines = localStorage.getItem(MARKS_KEY) !== "off";
    saveBeforeSend = localStorage.getItem(SAVE_FIRST_KEY) !== "off";
    openFilesIn = localStorage.getItem(FILES_KEY) === "single" ? "single" : "tabs";
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
  return {
    theme,
    widths,
    showUsage,
    showThinkingExpanded,
    openFilesIn,
    markVibeLines,
    saveBeforeSend,
    openHtmlMarkdownAs,
  };
}

/** Stores the theme and widths; failures are ignored. */
export function savePersisted(
  state: Pick<
    ShellState,
    | "theme"
    | "widths"
    | "showUsage"
    | "showThinkingExpanded"
    | "openFilesIn"
    | "markVibeLines"
    | "saveBeforeSend"
    | "openHtmlMarkdownAs"
  >,
): void {
  try {
    localStorage.setItem(FILES_KEY, state.openFilesIn);
    localStorage.setItem(THEME_KEY, state.theme);
    localStorage.setItem(WIDTHS_KEY, JSON.stringify(state.widths));
    localStorage.setItem(USAGE_KEY, state.showUsage ? "on" : "off");
    localStorage.setItem(THINKING_KEY, state.showThinkingExpanded ? "on" : "off");
    localStorage.setItem(MARKS_KEY, state.markVibeLines ? "on" : "off");
    localStorage.setItem(SAVE_FIRST_KEY, state.saveBeforeSend ? "on" : "off");
    localStorage.setItem(OPEN_AS_KEY, state.openHtmlMarkdownAs);
  } catch {
    // Not persisted; the app keeps working.
  }
}

/** The theme to start with: the stored choice, else dark (the product default, tokens.css). */
export function startingTheme(stored: Theme | null): Theme {
  return stored ?? "dark";
}
