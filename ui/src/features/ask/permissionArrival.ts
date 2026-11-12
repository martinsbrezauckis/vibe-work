/** Tracks keyboard activity for permission arrival; the shell starts it and AskCard waits for idle typing. */
import { useEffect, useState } from "react";

const TYPING_MS = 1000;
export const KEY_GRACE_MS = 500;
let lastKey = -Infinity;

/**
 * Milliseconds on a monotonic clock. Never the wall clock: a time-sync step back after a card appeared made its grace
 * read "just shown" for a second or more, and every keyboard or accessibility press meanwhile was silently dropped.
 */
const now = (): number => performance.now();
let users = 0;
const listeners = new Set<() => void>();

function key(): void {
  lastKey = now();
  for (const listener of listeners) listener();
}

/** Records activity before a permission exists, releasing the window listener with its last caller. */
export function startPromptActivity(): () => void {
  if (users++ === 0) window.addEventListener("keydown", key, true);
  return () => {
    if (--users === 0) {
      window.removeEventListener("keydown", key, true);
      lastKey = -Infinity;
    }
  };
}

/**
 * A mounted permission waits for idle typing once (`enabled`); keys are ignored briefly after any card becomes visible.
 */
export function usePermissionArrival(enabled: boolean): {
  visible: boolean;
  keysReady: () => boolean;
} {
  const [shownAt, setShownAt] = useState<number | null>(() =>
    !enabled || now() - lastKey >= TYPING_MS ? now() : null,
  );
  useEffect(startPromptActivity, []);
  useEffect(() => {
    if (shownAt !== null) return;
    let timer: ReturnType<typeof setTimeout>;
    const wait = () => {
      clearTimeout(timer);
      timer = setTimeout(
        () => {
          setShownAt(now());
        },
        Math.max(0, lastKey + TYPING_MS - now()),
      );
    };
    listeners.add(wait);
    wait();
    return () => {
      clearTimeout(timer);
      listeners.delete(wait);
    };
  }, [shownAt]);
  return {
    visible: shownAt !== null,
    // The grace after the card shows holds for every card, also one that does not wait for idle typing (a question).
    keysReady: () => shownAt !== null && now() - shownAt >= KEY_GRACE_MS,
  };
}
