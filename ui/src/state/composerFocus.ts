/**
 * Asks a composer's text box for the keyboard focus, from anywhere (New chat focuses the new-chat
 * composer; a chat started from it focuses the new session's composer). The composer may not be mounted yet (it
 * loads lazily and mounts after the view switches), so a request waits until that composer registers.
 * Depends on: nothing. Called by: features/shell/newChat and state/composer (requests), features/composer/Composer
 * (registers).
 * Invariants: one pending request per composer key; a request is consumed once (a later mount does not steal the
 * focus again) and lapses after `PENDING_MS` (a composer that mounts much later was not what the user asked for);
 * registering returns its own removal.
 */

/** How long a request waits for its composer to mount (ms): the lazy chunk and a session start fit well inside. */
const PENDING_MS = 5000;

const focusers = new Map<string, () => void>();
/** Pending requests: composer key → when it was asked (ms). */
const pending = new Map<string, number>();

/** Focuses the text box of `key` (a session id, or the new-chat key) now, or as soon as it mounts. */
export function requestComposerFocus(key: string): void {
  const focus = focusers.get(key);
  if (focus === undefined) {
    pending.set(key, Date.now());
    return;
  }
  pending.delete(key);
  focus();
}

/** Registers the text box of `key`; a pending request for it is served now. Returns the removal. */
export function registerComposerFocus(key: string, focus: () => void): () => void {
  focusers.set(key, focus);
  const asked = pending.get(key);
  pending.delete(key);
  if (asked !== undefined && Date.now() - asked <= PENDING_MS) focus();
  return () => {
    if (focusers.get(key) === focus) focusers.delete(key);
  };
}
