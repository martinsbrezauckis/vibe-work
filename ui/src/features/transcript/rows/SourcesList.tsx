/**
 * A list of web sources: each source is a button showing the page title and the host it
 * goes to, with the full URL as its tooltip; a click opens the link dialog (the canonical URL in full,
 * then "Open in browser" through the core). Used in a web search step and in the answer's side list.
 * Depends on: AppContext (transcript store), model/sources. Called by: StepBody, SourcesPanel.
 * Invariants: render only; titles are untrusted text (text nodes); nothing is fetched; a source whose
 * URL cannot be opened says so instead of a host, and the dialog explains why when clicked.
 */
import type { ReactElement } from "react";

import { useApp } from "../../shell/AppContext";
import type { Source } from "../model/sources";

/** The sources as a list of link buttons. */
export function SourcesList({
  sources,
  label,
}: {
  sources: readonly Source[];
  label: string;
}): ReactElement {
  const { transcript } = useApp();
  return (
    <ol className="sources" aria-label={label}>
      {sources.map((source) => (
        <li key={source.url}>
          <button
            type="button"
            className="source"
            title={`Review source: ${source.title} (${source.host ?? "cannot be opened"})`}
            aria-label={`Review source: ${source.title} (${source.host ?? "cannot be opened"})`}
            data-source-url={source.url}
            onClick={() => {
              transcript.askLink(source.url);
            }}
          >
            <span className="source-title">{source.title}</span>
            <span className="source-host mono">{source.host ?? "cannot be opened"}</span>
          </button>
        </li>
      ))}
    </ol>
  );
}
