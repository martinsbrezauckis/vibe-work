/**
 * The shell's own UI state (not the core's): selected session, side panel per session, remembered
 * panel widths, theme, list visibility and overlays, with the pure reducers that change it.
 * Reducers manage tab opening, closing and list visibility.
 * Depends on: nothing. Called by: shellStore (binds the reducers), panelLayout, components via
 * the store, tests.
 * Invariants: every reducer returns a new state and never mutates; a panel exists per session key
 * (`"new"` for the new-session view), closed by default; opening a tab adds it once and makes it
 * active; the list collapses for wide tabs unless the user pinned it (see `ListChoice`).
 */
import { type EditorLayout, editorLayoutOf, swapEditorSlot, unfoldChat } from "./editorLayout";
import { term } from "../vocabulary";

/**
 * The side panel's tabs: Project (the project handbook), Plan (the plan checklist,
 * with steps beside the chat), Browser, Changes, Terminal, File (a file the chat mentions;
 * offered only once a file was opened).
 */
export type TabKind = "project" | "plan" | "browser" | "git" | "changes" | "terminal" | "file";

/** Tab metadata: label and whether the tab wants room (collapses the list, compacts the chat). */
export const TAB_META: Readonly<Record<TabKind, { label: string; size: "side" | "wide" }>> = {
  project: { label: term("workNames", "projectTab").name, size: "side" },
  plan: { label: "Plan", size: "side" },
  browser: { label: "Browser", size: "wide" },
  git: { label: "Files", size: "side" },
  changes: { label: "Changes", size: "wide" },
  terminal: { label: "Terminal", size: "wide" },
  file: { label: "File", size: "side" },
};

/** Tab order everywhere (header buttons, "add tab"). */
export const TAB_KINDS: readonly TabKind[] = [
  "project",
  "plan",
  "browser",
  "git",
  "changes",
  "terminal",
  "file",
];

/** One side panel. */
export interface PanelState {
  open: boolean;
  tabs: readonly TabKind[];
  active: TabKind | null;
}

/**
 * Who decides whether the session list shows. `auto`: shown unless a wide tab is open;
 * `shown`: the user asked for it (rail click or toggle), a wide tab shrinks instead of collapsing
 * it; `hidden`: the user hid it.
 */
export type ListChoice = "auto" | "shown" | "hidden";

/**
 * Who chose the new-session folder: the native picker (the only source the core counts as
 * "picked"), or a Debug API client through `ui/worktrees folder`. Commands the new-session view sends for a
 * `debug-api` folder carry the Debug API origin, so the core asks the native dialog where it would for the
 * client itself (a later click must not lend the client the window's authority). `session` is a folder the New chat
 * view took from the session the user left (or the newest one); like `picker` it starts with the window's origin
 * (a plain session there, no trust grant: what "New session here" does), but the core never counts it as picked.
 */
export type NewFolderBy = "picker" | "debug-api" | "session";

/** Dialogs layered over the shell; Settings uses a page (`SettingsView`). */
export type Overlay = "palette" | null;

/**
 * The Settings page: the rail's cog is active, the list column lists the categories, the
 * content area shows one category. It is a layer over whatever was on screen; closing it shows that view again
 * unchanged (the session, its scroll, the Projects overview or Tools page stay as they were underneath).
 * `category` is remembered while closed (the page reopens where it was). `query` is the search inside Settings
 * ("In: Settings"); `target` is a setting id to scroll to and light up once (from a search hit), cleared after.
 */
interface SettingsView {
  open: boolean;
  category: string;
  query: string;
  target: string | null;
}

/** Theme names (tokens.css: dark is the default, light via `data-theme`). */
export type Theme = "dark" | "light";

/** Everything the shell remembers about its own screen. */
export interface ShellState {
  theme: Theme;
  /** Each session keeps its editor split and shared slot. */
  editorLayouts: Readonly<Record<string, EditorLayout>>;
  /** Selected session id; `null` is the new-session (welcome) view. */
  selected: string | null;
  list: ListChoice;
  /** Panels by session id, `"new"` for the new-session view. */
  panels: Readonly<Record<string, PanelState>>;
  /** Widths the user dragged to, per tab kind (remembered across sessions and restarts). */
  widths: Readonly<Partial<Record<TabKind, number>>>;
  /** The panel takes the whole window. */
  maximized: boolean;
  /** Project ids (full `cwd` for folder groups) collapsed in the list, including during filtering. */
  collapsed: Readonly<Record<string, true>>;
  /** Parent sessions whose child (subagent) sessions are shown in the list. */
  expanded: Readonly<Record<string, true>>;
  /**
   * The Answers panel under a handbook page shows every control (not only its one line). One choice for the window,
   * not per page; not saved across restarts.
   */
  answersShown: boolean;
  /** The folder for the next new session, or `null`. */
  newFolder: string | null;
  /** Who chose `newFolder` (`null` while there is none). */
  newFolderBy: NewFolderBy | null;
  /**
   * The core's owner id for what is open on the New chat screen (terminals, browser) before the first message;
   * `null` until the core gave one. Internal: never shown.
   */
  newChatOwner: string | null;
  filter: string;
  overlay: Overlay;
  /** The status bar shows the open session's context readout (Settings › Appearance, default on). */
  showUsage: boolean;
  /** Presentation of open files in the side panel. */
  openFilesIn: "tabs" | "single";
  /** Show thinking blocks and their containing steps groups by default (Settings › Appearance). */
  showThinkingExpanded: boolean;
  /** Mark the lines Vibe changes in open files (Settings › Appearance, default on). */
  markVibeLines: boolean;
  /**
   * How HTML and Markdown files open in Separate tabs (Settings › Appearance): `preview` (default), `code`, or `side`
   * (code with its preview beside, the two-editor split).
   */
  openHtmlMarkdownAs: "preview" | "code" | "side";
  /** Save edited open files when a message is sent (Settings › General, default on). */
  saveBeforeSend: boolean;
  /** The words the palette opens with (a search widened from a scoped field carries its words over). */
  paletteQuery: string;
  /** The palette starts in its Files scope for quick open. */
  paletteFiles: boolean;
  settings: SettingsView;
  /**
   * The session list as an overlay drawer. Only meaningful in narrow windows (the layout says
   * `drawer`), where the list has no column of its own; closed whenever a session is picked.
   */
  drawer: boolean;
}

/** The key of the panel that belongs to the current view. */
export function panelKey(state: Pick<ShellState, "selected">): string {
  return state.selected ?? "new";
}

const CLOSED: PanelState = { open: false, tabs: [], active: null };

/** The current view's panel (closed when never opened). */
export function panelOf(state: ShellState): PanelState {
  return state.panels[panelKey(state)] ?? CLOSED;
}

/** The state at startup. */
export function initialShell(
  theme: Theme,
  widths: ShellState["widths"] = {},
  showUsage = true,
  showThinkingExpanded = true,
): ShellState {
  return {
    theme,
    editorLayouts: {},
    selected: null,
    list: "auto",
    panels: {},
    widths,
    maximized: false,
    collapsed: {},
    expanded: {},
    answersShown: false,
    newFolder: null,
    newFolderBy: null,
    newChatOwner: null,
    filter: "",
    overlay: null,
    paletteQuery: "",
    paletteFiles: false,
    showUsage,
    openFilesIn: "tabs",
    showThinkingExpanded,
    markVibeLines: true,
    saveBeforeSend: true,
    openHtmlMarkdownAs: "preview",
    settings: { open: false, category: "general", query: "", target: null },
    drawer: false,
  };
}

function withPanel(state: ShellState, panel: PanelState): ShellState {
  const next: ShellState = { ...state, panels: { ...state.panels, [panelKey(state)]: panel } };
  // Closing the panel ends a "pinned" list (the next tab decides again whether it fits).
  if (!panel.open)
    return { ...next, maximized: false, list: state.list === "shown" ? "auto" : state.list };
  return next;
}

/** Opens `kind` (adding it once) and makes it the active tab. Ends a pinned list's hold on the room: where list,
 * chat and panel do not all fit, the panel shows again and the list folds (`computeLayout`). */
export function openTab(state: ShellState, kind: TabKind): ShellState {
  const panel = panelOf(state);
  const tabs = panel.tabs.includes(kind) ? panel.tabs : [...panel.tabs, kind];
  const next = withPanel(state, { open: true, tabs, active: kind });
  return next.list === "shown" ? { ...next, list: "auto" } : next;
}

/** Closes one tab; the last tab closing closes the panel. */
export function closeTab(state: ShellState, kind: TabKind): ShellState {
  const panel = panelOf(state);
  const tabs = panel.tabs.filter((t) => t !== kind);
  const active = panel.active === kind ? (tabs[tabs.length - 1] ?? null) : panel.active;
  return withPanel(state, { open: tabs.length > 0, tabs, active });
}

/** Closes the panel and all its tabs. */
export function closePanel(state: ShellState): ShellState {
  return withPanel(state, CLOSED);
}

/** The header buttons: a click on the active tab closes it, otherwise opens it. */
export function toggleTab(state: ShellState, kind: TabKind): ShellState {
  const panel = panelOf(state);
  return panel.open && panel.active === kind ? closeTab(state, kind) : openTab(state, kind);
}

/** Remembers a dragged width for a tab kind. */
export function setWidth(state: ShellState, kind: TabKind, width: number): ShellState {
  return { ...state, widths: { ...state.widths, [kind]: Math.round(width) } };
}

/** The list button: hide it when visible, pin it shown when not. */
export function toggleList(state: ShellState, visibleNow: boolean): ShellState {
  if (editorLayoutOf(state).split && panelOf(state).open) return swapEditorSlot(state);
  return { ...state, list: visibleNow ? "hidden" : "shown" };
}

/** A rail click always shows the list, even over a wide tab (the panel shrinks). */
export function showList(state: ShellState): ShellState {
  if (editorLayoutOf(state).split && panelOf(state).open)
    return {
      ...state,
      maximized: false,
      editorLayouts: {
        ...state.editorLayouts,
        [panelKey(state)]: { ...editorLayoutOf(state), slot: "list", chatFolded: false },
      },
    };
  // Also ends a maximized panel (the sessions strip is one of its folded columns).
  return { ...state, list: "shown", maximized: false };
}

/**
 * Selects a session (or the new-session view with `null`). Leaves panel-max. The Settings page stays open: the
 * session underneath changes; callers that go TO a session (the palette, the
 * setup screen) close Settings themselves.
 */
export function selectSession(state: ShellState, id: string | null): ShellState {
  return unfoldChat({ ...state, selected: id, maximized: false, drawer: false });
}

/** Toggles the panel's maximized state. */
export function toggleMaximized(state: ShellState): ShellState {
  return { ...state, maximized: !state.maximized };
}

/** Sets the theme. */
export function setTheme(state: ShellState, theme: Theme): ShellState {
  return { ...state, theme };
}

/** Shows or hides the status bar's usage readout. */
export function setShowUsage(state: ShellState, show: boolean): ShellState {
  return { ...state, showUsage: show };
}

/** Sets the default disclosure of thinking blocks in the transcript. */
export function setShowThinkingExpanded(state: ShellState, show: boolean): ShellState {
  return { ...state, showThinkingExpanded: show };
}

/** Turns the marks on lines Vibe changed in open files on or off. */
export function setMarkVibeLines(state: ShellState, on: boolean): ShellState {
  return { ...state, markVibeLines: on };
}

/** Sets how HTML and Markdown files open. */
export function setOpenHtmlMarkdownAs(
  state: ShellState,
  openHtmlMarkdownAs: ShellState["openHtmlMarkdownAs"],
): ShellState {
  return { ...state, openHtmlMarkdownAs };
}

/** Turns saving edited open files before a message is sent on or off. */
export function setSaveBeforeSend(state: ShellState, on: boolean): ShellState {
  return { ...state, saveBeforeSend: on };
}

/** Sets the session filter text. */
export function setFilter(state: ShellState, filter: string): ShellState {
  return { ...state, filter };
}

/** Shows or hides an overlay dialog; the palette opens with `query` typed (default empty). */
export function setOverlay(
  state: ShellState,
  overlay: Overlay,
  query = "",
  filesOnly = false,
): ShellState {
  return {
    ...state,
    overlay,
    paletteQuery: overlay === "palette" ? query : "",
    paletteFiles: filesOnly,
  };
}

/**
 * Opens the Settings page: at `category` (else the one it showed last) and, with `target`, at that setting.
 * Closes the palette and the list drawer: the page has the column to itself.
 */
export function openSettings(state: ShellState, category?: string, target?: string): ShellState {
  return {
    ...state,
    overlay: null,
    drawer: false,
    settings: {
      open: true,
      category: category ?? state.settings.category,
      query: target === undefined ? state.settings.query : "",
      target: target ?? null,
    },
  };
}

/** Closes the Settings page (the view underneath shows again); the search is forgotten, the category kept. */
export function closeSettings(state: ShellState): ShellState {
  return { ...state, settings: { ...state.settings, open: false, query: "", target: null } };
}

/** Shows one category (clears the search); with `target`, scrolls to that setting and lights it up. */
export function setSettingsCategory(
  state: ShellState,
  category: string,
  target?: string,
): ShellState {
  return {
    ...state,
    drawer: false,
    settings: { ...state.settings, category, query: "", target: target ?? null },
  };
}

/** The text of the search inside Settings. */
export function setSettingsQuery(state: ShellState, query: string): ShellState {
  return { ...state, settings: { ...state.settings, query } };
}

/** Forgets the one-shot `target` (after the page scrolled to it and lit it). */
export function clearSettingsTarget(state: ShellState): ShellState {
  return state.settings.target === null
    ? state
    : { ...state, settings: { ...state.settings, target: null } };
}

/** Collapses or expands one project id (folder path before filing) in the list. */
export function toggleProject(state: ShellState, project: string): ShellState {
  const collapsed = { ...state.collapsed };
  if (collapsed[project] === true) Reflect.deleteProperty(collapsed, project);
  else collapsed[project] = true;
  return { ...state, collapsed };
}

/** Expands or collapses the child sessions of one parent. */
export function toggleExpanded(state: ShellState, id: string): ShellState {
  const expanded = { ...state.expanded };
  if (expanded[id] === true) Reflect.deleteProperty(expanded, id);
  else expanded[id] = true;
  return { ...state, expanded };
}

/** Shows the Answers panel's controls (`true`) or only its one line. */
export function setAnswersShown(state: ShellState, shown: boolean): ShellState {
  return state.answersShown === shown ? state : { ...state, answersShown: shown };
}

/** Remembers the folder for the next new session and who chose it (`null` folder: forget both). */
export function setNewFolder(
  state: ShellState,
  folder: string | null,
  by: NewFolderBy,
): ShellState {
  return { ...state, newFolder: folder, newFolderBy: folder === null ? null : by };
}

/** The core's owner id for the New chat screen. */
export function setNewChatOwner(state: ShellState, owner: string | null): ShellState {
  return state.newChatOwner === owner ? state : { ...state, newChatOwner: owner };
}

/**
 * Who the shown side panel's terminals and browser belong to: the selected session, else the New chat screen's
 * owner (`null` until the core gave one).
 */
export function panelOwner(state: Pick<ShellState, "selected" | "newChatOwner">): string | null {
  return state.selected ?? state.newChatOwner;
}

/**
 * The first message started `sessionId`: the New chat screen's panel and editor layout become the session's, so
 * the same tabs stay open (the New chat screen starts empty again). `filesKey` is the editor key the screen used.
 */
export function handOverNewChat(
  state: ShellState,
  sessionId: string,
  filesKey: string | null,
): ShellState {
  const panels = { ...state.panels };
  const panel = panels["new"];
  Reflect.deleteProperty(panels, "new");
  if (panel !== undefined) panels[sessionId] = panel;
  const layouts = { ...state.editorLayouts };
  const layout = layouts["new"] ?? (filesKey === null ? undefined : layouts[filesKey]);
  Reflect.deleteProperty(layouts, "new");
  if (filesKey !== null) Reflect.deleteProperty(layouts, filesKey);
  if (layout !== undefined) layouts[sessionId] = layout;
  return { ...state, panels, editorLayouts: layouts };
}

/** Opens or closes the list drawer (narrow windows). */
export function setDrawer(state: ShellState, open: boolean): ShellState {
  return { ...state, drawer: open };
}

/** Sets the presentation of open files without closing their buffers. */
export function setOpenFilesIn(
  state: ShellState,
  openFilesIn: ShellState["openFilesIn"],
): ShellState {
  return { ...state, openFilesIn };
}

/** Keeps a working-surface tab first without changing the active view. */
export function pinTab(state: ShellState, kind: TabKind, id = panelKey(state)): ShellState {
  const panel = state.panels[id];
  if (!panel || panel.tabs[0] === kind) return state;
  return {
    ...state,
    panels: {
      ...state.panels,
      [id]: { ...panel, tabs: [kind, ...panel.tabs.filter((k) => k !== kind)] },
    },
  };
}
