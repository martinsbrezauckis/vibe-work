/**
 * The session index: how `session/list` pages and the sessions the app holds live become one set
 * of entries (history lives in Vibe's store, shared with the CLI; the app keeps no copy
 * of its own). Pure reducers over `{sessions, index}`; the controller (`coreStore.ts`) fetches the
 * pages and calls them.
 * Depends on: protocol/core types, sessionSummary (`SessionEntry`). Called by: coreModel, coreStore.
 * Invariants:
 * - A loaded entry (`state !== null`) keeps its live state; a listing never overwrites it (the core
 *   already hands back its live `session` object in the rows).
 * - A listing-only entry is replaced by the newest row for that id.
 * - A refresh that reached the end of the listing (`nextCursor === null`) drops listing-only
 *   entries Vibe no longer lists (deleted in the CLI or elsewhere); a partial refresh never drops
 *   anything, because an older session may simply sit on a page that was not fetched this time.
 * - `localOnly` mirrors the core: true until Vibe lists the session.
 */
import type { JsonValue, SessionRowView } from "../protocol/core";
import type { SessionEntry } from "./sessionSummary";

/** Paging and loading state of the index. */
export interface IndexState {
  /** How many pages the last refresh covered (a refresh re-reads that many). */
  pages: number;
  /** Cursor for the next page; `null` when the listing is complete. */
  nextCursor: string | null;
  /** Archived sessions are requested from Vibe (the archive filter is on). */
  includeArchived: boolean;
  /** `continueSessionId` of the first page: what `vibe --continue` resumes ("Last in CLI"). */
  continueSessionId: string | null;
  /** A page fetch is in flight. */
  loading: boolean;
  /** The first listing arrived. */
  loaded: boolean;
  /** The last listing failed (text for the list note), else `null`. */
  error: string | null;
}

/** The index before any listing. */
export const initialIndex: IndexState = {
  pages: 0,
  nextCursor: null,
  includeArchived: false,
  continueSessionId: null,
  loading: false,
  loaded: false,
  error: null,
};

/** The slice of the core model the reducers work on. */
export interface IndexedSessions {
  sessions: Readonly<Record<string, SessionEntry>>;
  index: IndexState;
}

function isObject(value: JsonValue | undefined): value is Record<string, JsonValue | undefined> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The id inside a listing row, or `null` for a row without one. */
export function rowId(row: SessionRowView): string | null {
  const session = row.session;
  if (!isObject(session)) return null;
  const id = session["id"];
  return typeof id === "string" ? id : null;
}

function mergeRow(row: SessionRowView, known: SessionEntry | undefined): SessionEntry | null {
  const id = rowId(row);
  const session = row.session;
  if (id === null || !isObject(session)) return null;
  if (known?.state !== null && known !== undefined) {
    if (
      known.localOnly === row.localOnly &&
      (row.accessMode === undefined || known.accessMode === row.accessMode)
    )
      return known;
    return {
      ...known,
      localOnly: row.localOnly,
      ...(row.accessMode === undefined ? {} : { accessMode: row.accessMode }),
    };
  }
  return {
    id,
    session,
    state: null,
    localOnly: row.localOnly,
    ...(row.accessMode === undefined ? {} : { accessMode: row.accessMode }),
    openCallbackIds: known?.openCallbackIds ?? [],
  };
}

/** Merges `rows` into the sessions without removing anything (a page append, or a bare listing). */
export function mergeRows(
  sessions: IndexedSessions["sessions"],
  rows: readonly SessionRowView[],
): IndexedSessions["sessions"] {
  const next = { ...sessions };
  for (const row of rows) {
    const id = rowId(row);
    if (id === null) continue;
    const merged = mergeRow(row, next[id]);
    if (merged !== null) next[id] = merged;
  }
  return next;
}

/** One fetched run of pages. */
export interface ListingPass {
  rows: readonly SessionRowView[];
  nextCursor: string | null;
  /** From the first page of the pass; `undefined` keeps the current value (an appended page). */
  continueSessionId?: string | null;
  /** Pages in this pass. */
  pages: number;
  /** A refresh from the first page (true) or one more page appended (false). */
  refresh: boolean;
}

/** Applies a pass: merge, prune when a refresh reached the end, update the paging state. */
export function applyListingPass<S extends IndexedSessions>(state: S, pass: ListingPass): S {
  let sessions = mergeRows(state.sessions, pass.rows);
  if (pass.refresh && pass.nextCursor === null) {
    const listed = new Set(pass.rows.map(rowId));
    sessions = Object.fromEntries(
      Object.entries(sessions).filter(
        ([id, entry]) => entry.state !== null || entry.localOnly || listed.has(id),
      ),
    );
  }
  const index: IndexState = {
    ...state.index,
    pages: pass.refresh ? pass.pages : state.index.pages + pass.pages,
    nextCursor: pass.nextCursor,
    continueSessionId:
      pass.continueSessionId === undefined ? state.index.continueSessionId : pass.continueSessionId,
    loading: false,
    loaded: true,
    error: null,
  };
  return { ...state, sessions, index };
}

/** Marks a fetch as started or failed. */
export function indexLoading<S extends IndexedSessions>(state: S, loading: boolean): S {
  return { ...state, index: { ...state.index, loading } };
}

/** Records a failed listing. */
export function indexFailed<S extends IndexedSessions>(state: S, message: string): S {
  return { ...state, index: { ...state.index, loading: false, error: message } };
}

/** Changes the archive filter's request flag (the controller then refreshes). */
export function setIncludeArchived<S extends IndexedSessions>(state: S, include: boolean): S {
  return { ...state, index: { ...state.index, includeArchived: include } };
}

/**
 * Sets fields on a session's `session` object (and on `state.session` when loaded), for the result
 * of a row action (pin, archive, rename, mark as seen) so the screen follows without waiting for
 * a re-list. Vibe also publishes these changes for loaded sessions; setting the same values twice
 * is harmless.
 */
export function patchSessionFields<S extends IndexedSessions>(
  state: S,
  id: string,
  fields: Record<string, JsonValue>,
): S {
  const entry = state.sessions[id];
  if (entry === undefined) return state;
  const session = { ...entry.session, ...fields };
  const loaded = entry.state;
  const next: SessionEntry = {
    ...entry,
    session,
    state: loaded === null ? null : { ...loaded, session },
  };
  return { ...state, sessions: { ...state.sessions, [id]: next } };
}

/** Drops a session (deleted). */
export function removeSession<S extends IndexedSessions>(state: S, id: string): S {
  return {
    ...state,
    sessions: Object.fromEntries(Object.entries(state.sessions).filter(([key]) => key !== id)),
  };
}
