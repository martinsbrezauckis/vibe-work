/**
 * The shell store: `ShellState` in a store, with persistence of display preferences and widths and one
 * action object that binds the pure reducers. Components call `actions`, read through `useStore`.
 * Depends on: createStore, shellState, persist. Called by: main.tsx, Shell components, the
 * UI-control routes, tests.
 * Invariants: persistence runs only when the theme, widths or a display switch changed.
 */
import { createStore } from "./createStore";
import { loadPersisted, savePersisted, startingTheme } from "./persist";
import * as r from "./shellState";

/** Creates the shell store and its actions. Pass `persist: false` in tests. */
export function createShell(options: { persist?: boolean; theme?: r.Theme } = {}) {
  const stored =
    options.persist === false
      ? { theme: null, widths: {}, showUsage: true, showThinkingExpanded: true }
      : loadPersisted();
  const store = createStore<r.ShellState>(
    r.initialShell(
      options.theme ?? startingTheme(stored.theme),
      stored.widths,
      stored.showUsage,
      stored.showThinkingExpanded,
    ),
  );
  if (options.persist !== false) {
    let last = store.get();
    store.subscribe(() => {
      const now = store.get();
      if (
        now.theme !== last.theme ||
        now.widths !== last.widths ||
        now.showUsage !== last.showUsage ||
        now.showThinkingExpanded !== last.showThinkingExpanded
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
      openTab: act(r.openTab),
      closeTab: act(r.closeTab),
      closePanel: act(r.closePanel),
      toggleTab: act(r.toggleTab),
      setWidth: act(r.setWidth),
      toggleList: act(r.toggleList),
      showList: act(r.showList),
      selectSession: act(r.selectSession),
      toggleMaximized: act(r.toggleMaximized),
      setTheme: act(r.setTheme),
      toggleTheme: act((s: r.ShellState) => r.setTheme(s, s.theme === "dark" ? "light" : "dark")),
      setFilter: act(r.setFilter),
      setShowUsage: act(r.setShowUsage),
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
      setNewFolder: act(r.setNewFolder),
      toggleDrawer: act((s: r.ShellState) => r.setDrawer(s, !s.drawer)),
    },
  };
}

/** The shell store and actions the app uses. */
export type Shell = ReturnType<typeof createShell>;
