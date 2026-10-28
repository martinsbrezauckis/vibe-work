/**
 * Composer drafts: the unsent text of each session, kept when the user switches sessions and across a
 * restart.
 * The three pure helpers preserve draft behavior (an empty draft removes the key; an unchanged draft returns the
 * same object; an unknown id reads as ""), keyed by session id. Drafts are stored in `localStorage`,
 * validated on read and capped; storage failures are ignored. Their author is recorded (unknown = the Debug API's,
 * fail closed).
 * Adapted from ShellX.
 * Depends on: nothing. Called by: state/composerStore.
 * Invariants: pure functions never mutate their input; `loadDrafts` never throws and returns only
 * string values; a draft is pruned only for sessions known to be gone (the session list is paged, so a
 * session missing from the loaded page may still exist).
 */

/** Drafts by session id. */
export type Drafts = Readonly<Record<string, string>>;

/** `localStorage` key. */
export const DRAFTS_KEY = "vw.composerDrafts";
/** Most drafts kept (the oldest entries beyond it are dropped when saving). */
const MAX_DRAFTS = 200;
/** Longest draft kept in storage (a longer one is kept in memory only). */
const MAX_DRAFT_CHARS = 100_000;

/** The draft of `sessionId`, or "" (no session, or nothing typed). */
export function draftFor(drafts: Drafts, sessionId: string | null | undefined): string {
  return sessionId ? (drafts[sessionId] ?? "") : "";
}

/**
 * `drafts` with the draft of `sessionId` set to `next`. Returns the same object when nothing changes;
 * an empty `next` removes the key.
 */
export function withDraft(
  drafts: Drafts,
  sessionId: string | null | undefined,
  next: string,
): Drafts {
  if (!sessionId) return drafts;
  const current = drafts[sessionId] ?? "";
  if (current === next) return drafts;
  if (next === "") {
    if (!(sessionId in drafts)) return drafts;
    return Object.fromEntries(Object.entries(drafts).filter(([id]) => id !== sessionId));
  }
  return { ...drafts, [sessionId]: next };
}

/** `drafts` without the given session ids (same object when none of them has a draft). */
export function withoutDrafts(drafts: Drafts, gone: Iterable<string>): Drafts {
  const stale = new Set([...gone].filter((id) => id in drafts));
  if (stale.size === 0) return drafts;
  return Object.fromEntries(Object.entries(drafts).filter(([id]) => !stale.has(id)));
}

/** Who wrote a stored draft: the user, a Debug API client, or the app (a rewound message). */
export type DraftBy = "user" | "debug-api" | "literal";

/** What `loadDrafts` returns: the texts and, per session, who wrote them. */
export interface StoredDrafts {
  drafts: Drafts;
  by: Readonly<Record<string, DraftBy>>;
}

/**
 * Reads the stored drafts; anything malformed is skipped. Format 2 is `{version: 2, drafts: {id: {text,
 * by}}}`. Provenance must survive a restart (a Debug API draft reloaded as plain
 * text would run as the user's `!cmd`), so anything without a known `by` (format 1 was plain strings)
 * is loaded as the Debug API's: fail closed.
 */
export function loadDrafts(): StoredDrafts {
  const out: Record<string, string> = {};
  const by: Record<string, DraftBy> = {};
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(DRAFTS_KEY) ?? "{}");
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed))
      return { drafts: {}, by: {} };
    const record = parsed as Record<string, unknown>;
    const v2 =
      record["version"] === 2 && typeof record["drafts"] === "object" && record["drafts"] !== null;
    const entries = Object.entries(v2 ? (record["drafts"] as Record<string, unknown>) : record);
    for (const [id, value] of entries) {
      if (id === "" || (!v2 && id === "version")) continue;
      const item =
        typeof value === "object" && value !== null ? (value as Record<string, unknown>) : null;
      const text = v2 ? item?.["text"] : value;
      if (typeof text !== "string" || text === "") continue;
      const who = v2 ? item?.["by"] : undefined;
      out[id] = text;
      by[id] = who === "user" || who === "literal" || who === "debug-api" ? who : "debug-api";
    }
    return { drafts: out, by };
  } catch {
    return { drafts: {}, by: {} };
  }
}

/**
 * Stores the drafts with who wrote each (the newest `MAX_DRAFTS`, each at most `MAX_DRAFT_CHARS`); a
 * draft without an entry in `by` is the user's. Failures are ignored.
 */
export function saveDrafts(drafts: Drafts, by: Readonly<Record<string, DraftBy>> = {}): void {
  try {
    const kept = Object.entries(drafts)
      .filter(([, text]) => text.length <= MAX_DRAFT_CHARS)
      .slice(-MAX_DRAFTS)
      .map(([id, text]) => [id, { text, by: by[id] ?? "user" }] as const);
    localStorage.setItem(
      DRAFTS_KEY,
      JSON.stringify({ version: 2, drafts: Object.fromEntries(kept) }),
    );
  } catch {
    // Not stored; the draft stays in memory.
  }
}
