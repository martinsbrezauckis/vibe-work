/**
 * Session-copy notification thresholds. Depends on localStorage; called by worktrees.
 * Remembers only Vibe Work's last threshold, never writes Vibe's config or removes a copy.
 * Falling below a threshold rearms it; unavailable storage still deduplicates for this run.
 */
const KEY = "vw.worktreeNotifiedThreshold";

/** Creates the persistent 15, 25, 35, ... crossing rule. Tests may disable persistence. */
export function createWorktreeNotifications(persist = true) {
  let last = 0;
  if (persist) {
    try {
      const stored = Number(localStorage.getItem(KEY));
      if (Number.isSafeInteger(stored) && stored >= 15 && (stored - 15) % 10 === 0) last = stored;
    } catch {
      // Storage unavailable: remember crossings for this run.
    }
  }
  return {
    /** Returns a newly crossed threshold, or null. A downward crossing only rearms it. */
    observe(count: number): number | null {
      const threshold = count < 15 ? 0 : 15 + Math.floor((count - 15) / 10) * 10;
      if (threshold === last) return null;
      const notified = threshold > last ? threshold : null;
      last = threshold;
      if (persist) {
        try {
          localStorage.setItem(KEY, String(last));
        } catch {
          // The in-memory threshold still prevents repeated notices.
        }
      }
      return notified;
    },
  };
}
