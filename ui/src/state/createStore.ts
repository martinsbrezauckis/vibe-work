/**
 * A tiny observable store (about 30 lines instead of a dependency): `get`, `set`, `subscribe`, and
 * a React hook on top of `useSyncExternalStore`.
 * Depends on: react. Called by: every store in `state/` (core, shell, toasts).
 * Invariants: `set` with an unchanged reference does not notify; listeners run synchronously in
 * subscription order; a selector passed to `useStore` must return a referentially stable value
 * (a primitive or an existing subtree of the state), otherwise React re-renders forever.
 */
import { useSyncExternalStore } from "react";

/** A store holding one immutable state value. */
export interface Store<T> {
  /** The current state. */
  get(): T;
  /** Replaces the state (a value, or a function of the previous state) and notifies listeners. */
  set(next: T | ((previous: T) => T)): void;
  /** Registers a listener; returns the function that removes it. */
  subscribe(listener: () => void): () => void;
}

/** Creates a store with `initial` as its first state. */
export function createStore<T>(initial: T): Store<T> {
  let state = initial;
  const listeners = new Set<() => void>();
  return {
    get: () => state,
    set(next) {
      const value = typeof next === "function" ? (next as (previous: T) => T)(state) : next;
      if (Object.is(value, state)) return;
      state = value;
      for (const listener of [...listeners]) listener();
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

/** Subscribes a component to `select(state)`; re-renders only when the selected value changes. */
export function useStore<T, S>(store: Store<T>, select: (state: T) => S): S {
  return useSyncExternalStore(
    (listener) => store.subscribe(listener),
    () => select(store.get()),
  );
}
