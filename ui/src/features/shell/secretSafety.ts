/**
 * Secret fields and the Debug API's element routes. The one rule that stays when the Debug API
 * acts as the user: a stored secret's VALUE never leaves, including a secret the user is typing or has shown in
 * clear (the key and password fields of the setup and Settings screens).
 *
 * - Every secret field carries `data-secret` (`setup/fields.tsx` `SecretField`, the only secret input); a
 *   password input counts as secret too, marked or not ([`SECRET_SELECTOR`]).
 * - Decided on the RESULT, never on the selector text (commit review, "parser differential"): every query a route
 *   runs for a client's selector ([`queryNeutral`]) runs while each secret field holds an empty value and no
 *   `value` attribute, and the values are put back right after, synchronously (no event, no render in between).
 *   So no selector, however written (`:is()`, `:has()`, escapes, case, sibling combinators, value-state
 *   pseudo-classes), can make the answer depend on a secret's value: there is no yes/no oracle.
 * - Window screenshots blank every secret field ([`secretRects`]). The capture lock holds across a capture
 *   (commit review, "toctou"): while it is on, `SecretField` shows dots and its Show button is disabled; the
 *   Debug API locks before the capture and unlocks after it, compares the rectangles of both answers and
 *   discards the image when anything moved (vw-debug-api `window_routes.rs`). A lock that is never released ends
 *   by itself after [`LOCK_MAX_MS`].
 * Depends on: createStore. Called by: inspectRoutes, inputRoutes, setup/fields.
 */
import { createStore, useStore } from "../../state/createStore";

/** Elements whose content is a secret. */
export const SECRET_SELECTOR = '[data-secret], input[type="password"]';
/** Longest a capture lock lasts when nobody releases it. */
export const LOCK_MAX_MS = 15_000;

/** The capture lock: `epoch` counts lock changes (a re-check compares it). */
const lock = createStore<{ locked: boolean; epoch: number }>({ locked: false, epoch: 0 });
let releaseTimer: ReturnType<typeof setTimeout> | undefined;

/** Turns the capture lock on or off; returns the new epoch. */
export function setCaptureLock(on: boolean): number {
  clearTimeout(releaseTimer);
  lock.set((s) => ({ locked: on, epoch: s.epoch + 1 }));
  if (on)
    releaseTimer = setTimeout(() => {
      setCaptureLock(false);
    }, LOCK_MAX_MS);
  return lock.get().epoch;
}

/** The lock's state (for the `secrets` route). */
export function captureLock(): { locked: boolean; epoch: number } {
  return lock.get();
}

/** True while a capture is in progress (`SecretField` shows dots and disables Show). */
export function useCaptureLocked(): boolean {
  return useStore(lock, (s) => s.locked);
}

/** Runs `query` while every secret field holds an empty value (module docs), then puts the values back. */
export function queryNeutral<T>(query: () => T): T {
  const saved = [...document.querySelectorAll<HTMLInputElement>(SECRET_SELECTOR)].map((el) => ({
    el,
    value: el.value,
    attribute: el.getAttribute("value"),
  }));
  for (const { el } of saved) {
    el.value = "";
    el.removeAttribute("value");
  }
  try {
    return query();
  } finally {
    for (const { el, value, attribute } of saved) {
      el.value = value;
      if (attribute !== null) el.setAttribute("value", attribute);
    }
  }
}

/** The rectangles (CSS pixels) of the secret fields that are shown, and whether any shows its value in clear. */
export function secretRects(): {
  rects: { x: number; y: number; width: number; height: number }[];
  clear: number;
} {
  const shown = [...document.querySelectorAll(SECRET_SELECTOR)].filter((el) => {
    const r = el.getBoundingClientRect();
    // Settings keeps the underlying setup mounted but hidden; its fields still have rectangles.
    return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility === "visible";
  });
  return {
    rects: shown.map((el) => {
      const r = el.getBoundingClientRect();
      return { x: r.left, y: r.top, width: r.width, height: r.height };
    }),
    clear: shown.filter((el) => !(el instanceof HTMLInputElement && el.type === "password")).length,
  };
}
