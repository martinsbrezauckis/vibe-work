/**
 * Find in this conversation and find in this file: Ctrl F in a
 * conversation searches its messages, Ctrl F in the File tab searches the shown file. One small store for both bars.
 * Depends on: createStore. Called by: createApp, the transcript's find bar, the File tab's find bar, the palette
 * (a message result opens the conversation's bar at that message), the `ui/search` route.
 * Invariants: a bar belongs to one session (the conversation) or one session's File tab; `at` is an index into the
 * matches the bar computed (clamped by the bar, never by the store); `target` is a one-shot request to start at an
 * entry (a message result), cleared once the bar used it.
 */
import { createStore } from "./createStore";

/** One find bar's state. */
export interface FindBar {
  /** The session it searches in. */
  sessionId: string;
  query: string;
  /** The current match (index into the matches the bar shows). */
  at: number;
  /** Start at this entry (a message result), once. */
  target: string | null;
  /** Bumped by every open, so the input takes focus again. */
  opened: number;
  /** The message a result named is further back than the conversation shows (the bar says so). */
  older: boolean;
}

/** Both bars (`null` = closed). */
export interface FindState {
  conversation: FindBar | null;
  file: FindBar | null;
  step: { sessionId: string; stepId: string } | null;
}

/** Which bar. */
export type FindScope = "conversation" | "file";

/** Creates the store and its actions. */
export function createFind() {
  const store = createStore<FindState>({ conversation: null, file: null, step: null });
  const set = (scope: FindScope, update: (bar: FindBar) => FindBar) => {
    store.set((s) => {
      const bar = s[scope];
      return bar === null ? s : { ...s, [scope]: update(bar) };
    });
  };
  return {
    store,
    actions: {
      /** Requests the same transcript highlight as a search result, with focus on the step. */
      openStep(sessionId: string, stepId: string): void {
        store.set((s) => ({ ...s, step: { sessionId, stepId } }));
      },
      /** Opens the conversation's bar (with words carried over, and optionally a message to start at). */
      open(sessionId: string, query?: string, target: string | null = null): void {
        store.set((s) => ({
          ...s,
          conversation: {
            sessionId,
            query: query ?? (s.conversation?.sessionId === sessionId ? s.conversation.query : ""),
            at: 0,
            target,
            opened: (s.conversation?.opened ?? 0) + 1,
            older: false,
          },
        }));
      },
      /** Opens the File tab's bar for `sessionId`. */
      openFile(sessionId: string, query?: string): void {
        store.set((s) => ({
          ...s,
          file: {
            sessionId,
            query: query ?? (s.file?.sessionId === sessionId ? s.file.query : ""),
            at: 0,
            target: null,
            opened: (s.file?.opened ?? 0) + 1,
            older: false,
          },
        }));
      },
      setQuery(scope: FindScope, query: string): void {
        set(scope, (bar) => ({ ...bar, query, at: 0, target: null, older: false }));
      },
      /** Moves to the next (`1`) or previous (`-1`) match; `count` is how many the bar has (wraps around). */
      step(scope: FindScope, by: 1 | -1, count: number): void {
        if (count <= 0) return;
        set(scope, (bar) => ({ ...bar, at: (((bar.at + by) % count) + count) % count }));
      },
      /** Jumps to match `at` (the bar's target resolution). */
      jump(scope: FindScope, at: number, older = false): void {
        set(scope, (bar) => ({ ...bar, at, target: null, older }));
      },
      close(scope: FindScope): void {
        store.set((s) => ({ ...s, [scope]: null }));
      },
    },
  };
}

/** The find store type. */
export type Find = ReturnType<typeof createFind>;
