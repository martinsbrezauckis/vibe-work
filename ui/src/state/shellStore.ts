/**
 * The shell store: `ShellState` in a store, with persistence of display preferences and widths and one
 * action object that binds the pure reducers. Components call `actions`, read through `useStore`.
 * Depends on: createStore, shellState, persist. Called by: main.tsx, Shell components, the
 * UI-control routes, tests.
 * Invariants: persistence runs only when the theme, widths or a display switch changed.
 */
import { createStore } from "./createStore";
import { loadPersisted, savePersisted, startingTheme } from "./persist";
import * as e from "./editorLayout";
import * as r from "./shellState";

/** Creates the shell store and its actions. Pass `persist: false` in tests. */
export function createShell(options: { persist?: boolean; theme?: r.Theme } = {}) {
  const stored =
    options.persist === false
      ? {
          theme: null,
          widths: {},
          showUsage: true,
          showThinkingExpanded: true,
          openFilesIn: "tabs" as const,
          markVibeLines: true,
          saveBeforeSend: true,
          openHtmlMarkdownAs: "preview" as const,
        }
      : loadPersisted();
  const store = createStore<r.ShellState>({
    ...r.initialShell(
      options.theme ?? startingTheme(stored.theme),
      stored.widths,
      stored.showUsage,
      stored.showThinkingExpanded,
    ),
    openFilesIn: stored.openFilesIn,
    markVibeLines: stored.markVibeLines,
    saveBeforeSend: stored.saveBeforeSend,
    openHtmlMarkdownAs: stored.openHtmlMarkdownAs,
  });
  if (options.persist !== false) {
    let last = store.get();
    store.subscribe(() => {
      const now = store.get();
      if (
        now.openFilesIn !== last.openFilesIn ||
        now.theme !== last.theme ||
        now.widths !== last.widths ||
        now.showUsage !== last.showUsage ||
        now.showThinkingExpanded !== last.showThinkingExpanded ||
        now.markVibeLines !== last.markVibeLines ||
        now.saveBeforeSend !== last.saveBeforeSend ||
        now.openHtmlMarkdownAs !== last.openHtmlMarkdownAs
      )
        savePersisted(now);
      last = now;
    });
  }
  /** Turns a reducer `(state, ...args) => state` into an action that updates the store. */
  const act =
    <A extends unknown[]>(reduce: (state: r.ShellState, ...args: A) => r.ShellState) =>
    (...args: A): void => {
      store.set((state) => reduce(state, ...args));
    };
  return {
    store,
    actions: {
      setEditorLayout: act(e.setEditorLayout),
      swapEditorSlot: act(e.swapEditorSlot),
      foldChat: act(e.foldChat),
      unfoldChat: act(e.unfoldChat),
      pinTab: act(r.pinTab),
      openTab: act(r.openTab),
      closeTab: act(r.closeTab),
      closePanel: act(r.closePanel),
      toggleTab: act(r.toggleTab),
      setWidth: act(r.setWidth),
      toggleList: act(r.toggleList),
      showList: act(r.showList),
      selectSession: act(r.selectSession),
      setNewChatOwner: act(r.setNewChatOwner),
      handOverNewChat: act(r.handOverNewChat),
      toggleMaximized: act(r.toggleMaximized),
      setTheme: act(r.setTheme),
      toggleTheme: act((s: r.ShellState) => r.setTheme(s, s.theme === "dark" ? "light" : "dark")),
      setFilter: act(r.setFilter),
      setShowUsage: act(r.setShowUsage),
      setMarkVibeLines: act(r.setMarkVibeLines),
      setSaveBeforeSend: act(r.setSaveBeforeSend),
      setOpenHtmlMarkdownAs: act(r.setOpenHtmlMarkdownAs),
      setOpenFilesIn: act(r.setOpenFilesIn),
      setShowThinkingExpanded: act(r.setShowThinkingExpanded),
      setOverlay: act(r.setOverlay),
      openSettings: act(r.openSettings),
      closeSettings: act(r.closeSettings),
      setSettingsCategory: act(r.setSettingsCategory),
      setSettingsQuery: act(r.setSettingsQuery),
      clearSettingsTarget: act(r.clearSettingsTarget),
      toggleProject: act(r.toggleProject),
      setDrawer: act(r.setDrawer),
      toggleExpanded: act(r.toggleExpanded),
      setAnswersShown: act(r.setAnswersShown),
      setNewFolder: act(r.setNewFolder),
      toggleDrawer: act((s: r.ShellState) => r.setDrawer(s, !s.drawer)),
    },
  };
}

/** The shell store and actions the app uses. */
export type Shell = ReturnType<typeof createShell>;
