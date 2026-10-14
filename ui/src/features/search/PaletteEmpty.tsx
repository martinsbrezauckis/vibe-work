/**
 * Loading, failure and empty feedback for content searches, including when Settings still has a match.
 * Depends on: paletteModel, useCoreSearch. Called by: CommandPalette.
 * Invariant: a pending or incomplete content search is never reported as a completed search with no matches.
 */
import type { Filter } from "./paletteModel";
import type { CoreResults } from "./useCoreSearch";

/** Renders feedback when the searched scope has no visible rows. Actions have their own empty wording. */
export function PaletteEmpty({
  query,
  mode,
  filter,
  found,
  empty,
  contentEmpty,
}: {
  query: string;
  mode: "empty" | "text" | "action";
  filter: Filter;
  found: CoreResults;
  empty: boolean;
  contentEmpty: boolean;
}) {
  if (!empty && !(mode === "text" && filter === "all" && contentEmpty)) return null;
  const localOnly = mode === "text" && (filter === "setting" || filter === "session");
  const waiting = !localOnly && (found.loading || found.index?.filling === true);
  const error = localOnly ? null : (found.error ?? found.index?.error ?? null);
  const failed = error !== null;
  const title =
    mode === "action"
      ? `No actions match “${query}”`
      : mode === "empty"
        ? "No matches"
        : failed
          ? "Search could not finish"
          : waiting
            ? "Searching sessions, messages and files…"
            : filter === "all"
              ? `No matches in sessions, messages or files for “${query}”`
              : `No matches in ${filter === "setting" ? "settings" : `${filter}s`} for “${query}”`;
  return (
    <li className="pal-empty" role="presentation">
      <b>{title}</b>
      <span>
        {mode === "action"
          ? "Try another action name."
          : (error ??
            (waiting ? "Results appear as the search completes." : "Use / to search actions."))}
      </span>
    </li>
  );
}
