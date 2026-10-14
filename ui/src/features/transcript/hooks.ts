/**
 * Read hooks for the transcript views: the transcript UI store and the selected session's data.
 * Depends on: AppContext, createStore, state types. Called by: the transcript components.
 * Invariants: selectors return stable values (primitives or existing subtrees), see `useStore`.
 */
import { useStore } from "../../state/createStore";
import type { TranscriptUi } from "../../state/transcriptStore";
import { useApp } from "../shell/AppContext";

/** Selects from the transcript UI state. `select` must return a stable value. */
export function useTranscriptUi<S>(select: (state: TranscriptUi) => S): S {
  return useStore(useApp().transcript.store, select);
}
