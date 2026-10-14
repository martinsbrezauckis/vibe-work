/**
 * Read hooks for the composer views.
 * Depends on: AppContext, createStore, composerStore types. Called by: the composer components.
 * Invariants: selectors return stable values (primitives or existing subtrees), see `useStore`.
 */
import { useStore } from "../../state/createStore";
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
