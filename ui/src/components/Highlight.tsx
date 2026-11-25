/**
 * Text with the words of a search query marked (`<mark>`), for search results and the rows a search found
 * (every result shows its matched words).
 * Depends on: react, state/highlight. Called by: SettingsNav, SettingsPage, the palette.
 */
import { Fragment, type ReactElement } from "react";

import { highlightRuns } from "../state/highlight";

/** `text` with every word of `query` marked; plain text when the query is empty. */
export function Highlight({
  text,
  query,
  shortAtWordStart = false,
}: {
  text: string;
  query: string;
  /** A query of one or two characters marks only where a word starts (Settings search). */
  shortAtWordStart?: boolean;
}): ReactElement {
  if (query.trim() === "") return <>{text}</>;
  return (
    <>
      {highlightRuns(text, query, shortAtWordStart).map((run, i) =>
        run.hit ? <mark key={i}>{run.text}</mark> : <Fragment key={i}>{run.text}</Fragment>,
      )}
    </>
  );
}
