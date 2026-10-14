/**
 * Dropping a deleted session's entries from a store's per-session maps. Feature stores keep
 * state keyed by session id (a diff view, a preview list, file tabs, page hubs); when a session leaves the model
 * (`CoreController.onSessionsGone`) these entries go too, so a store never grows with sessions that no longer exist.
 * Depends on: nothing. Called by: the feature stores (changes, line comments, preview, pages, files).
 * Invariants: pure; maps not named and entries of other sessions are kept as they are (same object when nothing
 * changed, so subscribers see no change).
 */

/** `state` without the entries of `ids` in each per-session map named in `fields`. */
export function pruneSessions<S extends object>(
  state: S,
  fields: readonly (keyof S)[],
  ids: readonly string[],
): S {
  let next: S | null = null;
  for (const field of fields) {
    const map = state[field] as Readonly<Record<string, unknown>>;
    if (!ids.some((id) => id in map)) continue;
    const kept: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(map)) if (!ids.includes(key)) kept[key] = value;
    next = { ...(next ?? state), [field]: kept };
  }
  return next ?? state;
}
