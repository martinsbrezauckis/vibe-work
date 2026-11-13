/**
 * Switching the theme without a flash (feel item 15): the change runs inside a view transition, so the
 * window cross-fades from one theme to the other in 200 ms (`::view-transition-*` timing in motion.css). The new
 * theme reaches the DOM inside the transition's callback (the `data-theme` attribute at once, React's render through
 * `flushSync`), so the transition's "after" picture is really the new theme. Where the API is missing (jsdom), on
 * Linux (starting a view transition ends WebKitGTK's web process and the window stops responding) or when reduced
 * motion is asked for, the change applies at once.
 * Depends on: react-dom (`flushSync`), the DOM. Called by: the title bar's theme button, Settings › Appearance.
 */
import { flushSync } from "react-dom";

import type { Theme } from "./shellState";

/** True in the Linux app (WebKitGTK), whose web process ends when a view transition starts. */
function linux(): boolean {
  const nav = typeof navigator === "undefined" ? undefined : navigator;
  const platform = `${nav?.platform ?? ""} ${nav?.userAgent ?? ""}`;
  return /linux/i.test(platform) && !/android/i.test(platform);
}

/** Sets the theme to `next` through `set`, cross-fading the window when that is possible and wanted. */
export function switchTheme(next: Theme, set: (theme: Theme) => void): void {
  const doc = document as Document & { startViewTransition?: (cb: () => void) => unknown };
  const reduced =
    document.documentElement.dataset["motion"] === "reduce" ||
    (typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  if (typeof doc.startViewTransition !== "function" || reduced || linux()) {
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
