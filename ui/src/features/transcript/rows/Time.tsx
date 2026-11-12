/**
 * Timestamps and durations for transcript rows: local `HH:MM` with the
 * full date and time in `title`, inside a `<time dateTime>`; a missing time renders nothing.
 * Adapted from ShellX.
 * Depends on: react. Called by: the message rows and the end-of-turn row.
 */
import type { ReactElement } from "react";

const pad = (n: number): string => String(n).padStart(2, "0");

/** `HH:MM` in local time. */
function fmtTime(ms: number): string {
  const d = new Date(ms);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/**
 * `850ms`, `3.2s`, `42s`, `2m05s` (ShellX's format).
 * Each unit is chosen from the already rounded value, so a duration just under a boundary carries
 * (239.6s is `4m00s`, never `3m60s`; 59.6s is `1m00s`, never `60s`).
 */
export function formatElapsed(ms: number): string {
  const whole = Math.round(ms);
  if (whole < 1000) return `${String(whole)}ms`;
  const tenths = Math.round(ms / 100);
  if (tenths < 100) return `${(tenths / 10).toFixed(1)}s`;
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${String(seconds)}s`;
  return `${String(Math.floor(seconds / 60))}m${pad(seconds % 60)}s`;
}

/** A row's time; nothing for a missing (0) time. */
export function Time({ ms }: { ms: number }): ReactElement | null {
  if (ms <= 0) return null;
  const date = new Date(ms);
  return (
    <time className="tx-time" dateTime={date.toISOString()} title={date.toLocaleString()}>
      {fmtTime(ms)}
    </time>
  );
}
