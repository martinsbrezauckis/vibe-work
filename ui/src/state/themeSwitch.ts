/**
 * Switching the theme without a flash (feel item 15): the change runs inside a view transition, so the
 * window cross-fades from one theme to the other in 200 ms (`::view-transition-*` timing in motion.css). The new
 * theme reaches the DOM inside the transition's callback (the `data-theme` attribute at once, React's render through
 * `flushSync`), so the transition's "after" picture is really the new theme. Where the API is missing (older
 * WebKitGTK, jsdom) or reduced motion is asked for, the change applies at once.
 * Depends on: react-dom (`flushSync`), the DOM. Called by: the title bar's theme button, Settings › Appearance.
 */
import { flushSync } from "react-dom";

import type { Theme } from "./shellState";

/** Sets the theme to `next` through `set`, cross-fading the window when that is possible and wanted. */
export function switchTheme(next: Theme, set: (theme: Theme) => void): void {
  const doc = document as Document & { startViewTransition?: (cb: () => void) => unknown };
  const reduced =
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (typeof doc.startViewTransition !== "function" || reduced) {
    set(next);
    return;
  }
  doc.startViewTransition(() => {
    document.documentElement.dataset["theme"] = next;
    flushSync(() => {
      set(next);
    });
  });
}
