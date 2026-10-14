/**
 * Timestamps and durations for transcript rows: local `HH:MM` with the
 * full date and time in `title`, inside a `<time dateTime>`; a missing time renders nothing.
 * Adapted from ShellX.
 * Depends on: react. Called by: the message rows and the end-of-turn row.
 */
import type { ReactElement } from "react";

const pad = (n: number): string => String(n).padStart(2, "0");

/** `HH:MM` in local time. */
export function fmtTime(ms: number): string {
  const d = new Date(ms);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** `850ms`, `3.2s`, `42s`, `2m05s` (ShellX's format). */
export function formatElapsed(ms: number): string {
  if (ms < 1000) return `${String(Math.round(ms))}ms`;
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)}s`;
  if (ms < 60_000) return `${String(Math.round(ms / 1000))}s`;
  const m = Math.floor(ms / 60_000);
  const s = Math.round((ms % 60_000) / 1000);
  return `${String(m)}m${pad(s)}s`;
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
