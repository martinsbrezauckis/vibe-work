/**
 * The "Right-click menus" switch ("Send to Vibe Work") and its one-time question, shared by Settings > General and
 * the row on the setup and start screens, so both show the same answer at once. One store per client.
 * Depends on: the client's `sendTo` (absent in browser dev mode: nothing is shown), createStore.
 * Called by: SendToSettings, SendToOffer, the `ui/settings` route.
 * Invariants: the menus change only through `set`; a failed change keeps the last confirmed view and shows the
 * host's sentence ("Could not add the menu entry: ... Nothing was changed.").
 */
import { useEffect } from "react";

import type { CoreClient, SendToRequest, SendToView } from "../protocol/client/types";
import { toCallError } from "../protocol/client/types";
import { createStore, useStore } from "./createStore";
import type { Store } from "./createStore";

/** What the switch and the row show. */
export interface SendToState {
  /** The host's view; `null` until read (or where the host has no switch). */
  view: SendToView | null;
  pending: boolean;
  error: string | null;
}

const stores = new WeakMap<CoreClient, Store<SendToState>>();

function storeOf(client: CoreClient): Store<SendToState> {
  let store = stores.get(client);
  if (store === undefined) {
    store = createStore<SendToState>({ view: null, pending: false, error: null });
    stores.set(client, store);
  }
  return store;
}

/** The shared state now (the `ui/settings` route reads it after a request). */
export function sendToState(client: CoreClient): SendToState {
  return storeOf(client).get();
}

/** Runs one request and keeps its answer; resolves to the new view, or `null` (with `error` set) on a failure. */
export async function sendToRequest(
  client: CoreClient,
  request: SendToRequest,
): Promise<SendToView | null> {
  const store = storeOf(client);
  if (client.sendTo === undefined) return null;
  store.set((s) => ({ ...s, pending: request.op !== "read", error: null }));
  try {
    const view = await client.sendTo(request);
    store.set({ view, pending: false, error: null });
    return view;
  } catch (thrown) {
    store.set((s) => ({ ...s, pending: false, error: toCallError(thrown).error.message }));
    return null;
  }
}

const EMPTY = createStore<SendToState>({ view: null, pending: false, error: null });

/** The switch for components; reads it again whenever one is shown (the installer or another window may have changed it). */
export function useSendTo(client: CoreClient | null): SendToState {
  const state = useStore(client === null ? EMPTY : storeOf(client), (s) => s);
  useEffect(() => {
    if (client !== null) void sendToRequest(client, { op: "read" });
  }, [client]);
  return state;
}

/** The current menu state, with the system's location when enabled. */
export function sendToStatus(view: SendToView): string {
  if (!view.on) return "Off";
  if (view.system === "windows" && view.underMoreOptions) return "On · Show more options";
  if (view.system === "macos") return "On · Finder and Services";
  if (view.system === "linux") return "On · Files and Dolphin";
  return "On";
}
