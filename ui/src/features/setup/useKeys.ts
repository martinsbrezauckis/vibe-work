/**
 * Read hook for the key screens: selects from the key controller's store, with a quiet fallback when
 * the app has no core (then there is nothing to set up).
 * Depends on: AppContext, state/keyStore, state/createStore. Called by: the setup and settings
 * components, Content.
 */
import { type Store, createStore, useStore } from "../../state/createStore";
import type { KeyUi } from "../../state/keyStore";
import { useApp } from "../shell/AppContext";

const NONE: Store<KeyUi> = createStore<KeyUi>({
  availability: "none",
  status: null,
  candidates: [],
  vibeKeys: [],
  mistralKeys: [],
  switching: null,
  busy: false,
  problem: null,
  test: { phase: "idle" },
  need: null,
  dismissed: false,
  setupDone: false,
  startupUnlock: false,
  removable: null,
  notice: null,
  resultPending: false,
});

/** Selects from the key state. `select` must return a stable value (see `useStore`). */
export function useKeyUi<S>(select: (state: KeyUi) => S): S {
  return useStore(useApp().keys?.store ?? NONE, select);
}
