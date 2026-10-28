/**
 * Hooks of the Connectors page: the page state from the connectors controller, and its
 * actions. Without a core the state is a fixed empty one and the actions are absent.
 * Depends on: AppContext, createStore, state/connectors. Called by: the connector views and the rail.
 */
import { useApp } from "../shell/AppContext";
import { createStore, useStore } from "../../state/createStore";
import type { Connectors, ConnectorsState } from "../../state/connectors";

const NONE = createStore<ConnectorsState>({
  open: false,
  view: null,
  loading: false,
  error: null,
  filter: "all",
  search: "",
  selected: null,
  addOpen: false,
  removing: null,
  addPrefill: null,
  signIn: null,
  prompts: [],
  note: null,
});

/** Subscribes to `select(state)` of the page (see `useStore` for the selector rule). */
export function useConnectors<S>(select: (state: ConnectorsState) => S): S {
  const { connectors } = useApp();
  return useStore(connectors?.store ?? NONE, select);
}

/** The window's own actions, or `null` without a core. */
export function useConnectorActions(): Connectors["actions"] | null {
  return useApp().connectors?.actions ?? null;
}
