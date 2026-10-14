/**
 * Text with the words of a search query marked (`<mark>`), for search results and the rows a search found
 * (every result shows its matched words).
 * Depends on: features/settings/catalog (`highlightRuns`). Called by: SettingsNav, SettingsPage, the palette.
 */
import { Fragment, type ReactElement } from "react";

import { highlightRuns } from "../features/settings/catalog";

/** `text` with every word of `query` marked; plain text when the query is empty. */
export function Highlight({ text, query }: { text: string; query: string }): ReactElement {
  if (query.trim() === "") return <>{text}</>;
  return (
    <>
      {highlightRuns(text, query).map((run, i) =>
        run.hit ? <mark key={i}>{run.text}</mark> : <Fragment key={i}>{run.text}</Fragment>,
      )}
    </>
  );
}
