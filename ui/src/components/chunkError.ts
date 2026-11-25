/**
 * Plain words for an app part that could not be loaded. A bundler or browser message names a file address the person
 * cannot act on, so it becomes one sentence with a way out; any other reason stays as given.
 * Depends on: nothing. Called by: LazyChunk, state/voice (first-paint graph: keep this file tiny).
 */
export const CHUNK_HELP = "Try again; if it keeps failing, quit and reopen Vibe Work.";

// Chromium, Firefox and Safari word a failed dynamic import differently; none of them is for the person.
const BUNDLER_FAILURE =
  /dynamically imported module|importing a module script failed|failed to fetch module/i;

/** The reason to show for `error`: the help sentence for a bundler failure, the message otherwise, "" when blank. */
export function chunkFailureReason(error: unknown): string {
  const raw =
    error instanceof Error ? error.message.trim() : typeof error === "string" ? error.trim() : "";
  return BUNDLER_FAILURE.test(raw) ? CHUNK_HELP : raw;
}

/** The line the person reads: `Could not load <label>.` plus the way out, or the reason after a colon. */
export function chunkFailureLine(label: string, reason: string): string {
  return reason === CHUNK_HELP
    ? `Could not load ${label}. ${reason}`
    : `Could not load ${label}: ${reason}`;
}
