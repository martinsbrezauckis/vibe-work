/**
 * The "working" indicator (a pixel touch of our own): four pixels on a 2x2 grid, one lit at a time, stepping
 * round the square every 200 ms (CSS `steps()`, so it ticks like the Vibe CLI's frames instead of gliding). Orange is
 * the agent's voice: the walker shows only while Vibe works. Under `prefers-reduced-motion` it stops and shows four
 * half-lit pixels (tokens.css).
 * Sizes: `badge` (7 px, inside a session badge), `inline` (11 px, next to text), `large` (20 px, empty states).
 * Depends on: react, styles/shell.css (`.walker`). Called by: Badge, the composer's working row, the transcript.
 */
import type { ReactElement } from "react";

/** The indicator. Decorative: the text next to it ("Working", a badge label) carries the meaning. */
export function Walker({ size = "inline" }: { size?: "badge" | "inline" | "large" }): ReactElement {
  return (
    <span className={`walker walker-${size}`} aria-hidden="true">
      <i />
      <i />
      <i />
      <i />
    </span>
  );
}
