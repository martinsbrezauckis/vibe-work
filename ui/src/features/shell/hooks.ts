/**
 * Read hooks for shell components: selectors over the shell store and the core model, and the
 * derived values several components share (session summaries, layout). Components read through
 * these and never touch the stores' internals.
 * Depends on: AppContext, state/*. Called by: every shell component.
 */
import { useEffect, useMemo, useState } from "react";

import { type CoreState, initialCore } from "../../state/coreModel";
import { createStore, useStore } from "../../state/createStore";
import { type Layout, computeLayout } from "../../state/panelLayout";
import { type ShellState } from "../../state/shellState";
import { type SessionSummary, summarize } from "../../state/sessionSummary";
import { waitingSessions } from "../ask/waiting";
import { useApp } from "./AppContext";

/** The model shown when no client could be created (a production bundle in a plain browser). */
const NO_CORE = createStore<CoreState>({
  ...initialCore,
  server: { phase: "unavailable", reason: "This window has no connection to the Vibe core." },
});

/** Selects from the shell state. `select` must return a stable value (see `useStore`). */
export function useShellState<S>(select: (state: ShellState) => S): S {
  return useStore(useApp().shell.store, select);
}

/** Selects from the core model. `select` must return a stable value. */
export function useCoreState<S>(select: (state: CoreState) => S): S {
  return useStore(useApp().core?.store ?? NO_CORE, select);
}

/** Every known session as a summary (unsorted; use `buildList` for display order). */
export function useSessionSummaries(): readonly SessionSummary[] {
  const sessions = useCoreState((s) => s.sessions);
  const stuck = useCoreState((s) => s.stuck);
  // Joined into a string so the selection stays the same value while the set of waiting sessions does.
  const waiting = useCoreState((s) => [...waitingSessions(s)].join("\n"));
  return useMemo(() => {
    const ids = new Set(waiting.split("\n"));
    return Object.values(sessions).map((entry) => summarize(entry, stuck, ids));
  }, [sessions, stuck, waiting]);
}

/** The window's inner width, kept current. */
export function useViewportWidth(): number {
  const [width, setWidth] = useState(() => window.innerWidth);
  useEffect(() => {
    const onResize = () => {
      setWidth(window.innerWidth);
    };
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
    };
  }, []);
  return width;
}

/** The computed layout for the current shell state and window width. */
export function useLayout(): Layout {
  const state = useStore(useApp().shell.store, (s) => s);
  const width = useViewportWidth();
  return useMemo(() => computeLayout(state, width), [state, width]);
}
