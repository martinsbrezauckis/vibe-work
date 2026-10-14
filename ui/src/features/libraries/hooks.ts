/**
 * Hooks of the Libraries page: the page state from the libraries controller, and its actions. Without a
 * core the state is a fixed empty one and the actions are absent.
 * Depends on: AppContext, createStore, state/libraries. Called by: the library views and the rail.
 */
import { useApp } from "../shell/AppContext";
import { createStore, useStore } from "../../state/createStore";
import { INITIAL_LIBRARIES, type Libraries, type LibrariesState } from "../../state/libraries";

const NONE = createStore<LibrariesState>(INITIAL_LIBRARIES);

/** Subscribes to `select(state)` of the page (see `useStore` for the selector rule). */
export function useLibraries<S>(select: (state: LibrariesState) => S): S {
  const { libraries } = useApp();
  return useStore(libraries?.store ?? NONE, select);
}

/** The window's own actions, or `null` without a core. */
export function useLibraryActions(): Libraries["actions"] | null {
  return useApp().libraries?.actions ?? null;
}
