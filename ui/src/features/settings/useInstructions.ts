/**
 * Read hook for the personal instruction editor: selects from the shared controller's store (one per core).
 * Depends on: AppContext, state/instructions, state/createStore. Called by: DocEditor,
 * InstructionsSettings.
 */
import { type Store, createStore, useStore } from "../../state/createStore";
import {
  type Instructions,
  type InstructionsState,
  instructionsFor,
} from "../../state/instructions";
import { useApp } from "../shell/AppContext";

const NONE: Store<InstructionsState> = createStore<InstructionsState>({
  personal: {
    doc: null,
    draft: "",
    draftBy: "user",
    busy: "idle",
    error: null,
    stale: false,
    notice: null,
    reviewing: false,
  },
});

/** The shared controller, or `null` when the app has no core. */
export function useInstructionsController(): Instructions | null {
  const { core } = useApp();
  return core === null ? null : instructionsFor(core);
}

/** Selects from the instructions state. `select` must return a stable value (see `useStore`). */
export function useInstructionsState<S>(select: (state: InstructionsState) => S): S {
  const controller = useInstructionsController();
  return useStore(controller?.store ?? NONE, select);
}
