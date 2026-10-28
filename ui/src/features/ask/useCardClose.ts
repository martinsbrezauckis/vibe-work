/**
 * What happens when an open AskCard turns into its audit row in front of the user: focus moves
 * to the session's composer text box when it was inside the card (the card is gone), and the audit row's words are
 * announced once through the app's announcer.
 * Depends on: composer focus, the transcript store. Called by: the ask adapters.
 * Invariant: only a card this window showed open announces, once; an audit row drawn for an old answer is silent.
 * A card can leave the transcript before it draws its audit row: on the real Vibe an answered approval joins its tool
 * step in the turn's steps group (often collapsed) as soon as Vibe resolves the callback, which can come before the
 * core's answer record reaches the window. Then `late` (the card's own audit text, read from the stores) is watched
 * until the record is there and its words are said then.
 */
import { useCallback, useEffect, useRef } from "react";
import type { Store } from "../../state/createStore";
import { requestComposerFocus } from "../../state/composerFocus";
import { useApp } from "../shell/AppContext";

/** How long a card that left the transcript waits for its answer record before it stays silent (ms). */
const LATE_MS = 30_000;

/**
 * Says `late()`'s text once it is there: at once, or at the first change of `store` that brings it (within
 * `LATE_MS`).
 */
function announceWhenSettled<T>(
  store: Store<T>,
  late: () => string | null,
  say: (text: string) => void,
): void {
  const now = late();
  if (now !== null) {
    say(now);
    return;
  }
  let stop = (): void => {};
  const timer = setTimeout(() => {
    stop();
  }, LATE_MS);
  stop = store.subscribe(() => {
    const text = late();
    if (text === null) return;
    stop();
    clearTimeout(timer);
    say(text);
  });
}

/**
 * `closed` is true once the card shows its audit row (`text`). `late`, when given, reads the card's audit text from
 * the stores (null while the answer is not settled), for a card that leaves the transcript while still open. Returns the callback the card calls with whether the
 * focus is inside it (pass it to `AskCard`'s `onFocusInside`).
 */
export function useCardClose(
  closed: boolean,
  composerKey: string,
  text: string | null,
  late?: () => string | null,
): (inside: boolean) => void {
  const { core, transcript } = useApp();
  const focused = useRef(false);
  const wasOpen = useRef(false);
  const key = useRef(composerKey);
  const lateRef = useRef(late);
  const coreRef = useRef(core);
  const transcriptRef = useRef(transcript);
  useEffect(() => {
    key.current = composerKey;
    lateRef.current = late;
    coreRef.current = core;
    transcriptRef.current = transcript;
  }, [composerKey, late, core, transcript]);
  useEffect(() => {
    if (!closed) {
      wasOpen.current = true;
      return;
    }
    if (!wasOpen.current) return;
    wasOpen.current = false;
    if (text !== null) transcript.announce(text);
    if (focused.current) {
      focused.current = false;
      requestComposerFocus(composerKey);
    }
  }, [closed, composerKey, text, transcript]);
  // The row can also leave the transcript at once (Vibe resolved the callback): the focus still moves, and the audit
  // words are said when the answer's record arrives.
  useEffect(
    () => () => {
      if (!wasOpen.current) return;
      if (focused.current) requestComposerFocus(key.current);
      const compute = lateRef.current;
      const store = coreRef.current?.store;
      if (compute !== undefined && store !== undefined) {
        announceWhenSettled(store, compute, (words) => {
          transcriptRef.current.announce(words);
        });
      }
    },
    [],
  );
  return useCallback((inside: boolean) => {
    focused.current = inside;
  }, []);
}
