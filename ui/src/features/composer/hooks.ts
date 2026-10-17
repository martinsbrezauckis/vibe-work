/**
 * Read hooks for the composer views, and the clock the Loops list counts down with.
 * Depends on: AppContext, createStore, composerStore types. Called by: the composer components.
 * Invariants: selectors return stable values (primitives or existing subtrees), see `useStore`.
 */
import { useLayoutEffect } from "react";

import { createStore, useStore } from "../../state/createStore";
import type { ComposerState } from "../../state/composerStore";
import { type DocumentsState, NO_DOCUMENTS } from "../../state/documents";
import { useApp } from "../shell/AppContext";

/** Selects from the composer state. `select` must return a stable value. */
export function useComposerState<S>(select: (state: ComposerState) => S): S {
  return useStore(useApp().composer.store, select);
}

/** Selects from the documents state; `select` must return a stable value. */
export function useDocumentsState<S>(select: (state: DocumentsState) => S): S {
  return useStore(useApp().documents?.store ?? NO_DOCUMENTS, select);
}

/** Stable empty lists for selectors (a fresh `[]` per call would re-render forever). */
export const NONE: readonly never[] = Object.freeze([]);

/** The clock the Loops list counts down with: one value for every reader, renewed by `useNowSeconds`. */
const clock = createStore(Date.now() / 1000);

/**
 * The current time in epoch seconds, renewed every `everyMs` while `active` (the Loops list counts down to the
 * next run). Inactive: no timer runs and the value stays at the last renewal. The value is brought up to date before
 * the first paint after the hook turns active, so a list that appears late never shows an old countdown.
 */
export function useNowSeconds(active: boolean, everyMs = 1000): number {
  useLayoutEffect(() => {
    if (!active) return undefined;
    clock.set(Date.now() / 1000);
    const timer = setInterval(() => {
      clock.set(Date.now() / 1000);
    }, everyMs);
    return () => {
      clearInterval(timer);
    };
  }, [active, everyMs]);
  return useStore(clock, (now) => now);
}
