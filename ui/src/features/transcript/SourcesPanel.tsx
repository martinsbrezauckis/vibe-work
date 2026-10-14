/**
 * The side list of an answer's web sources, opened by the answer's "Sources (n)" chip.
 * Not modal: the conversation stays usable; focus moves to the list when it opens and back to its chip
 * when it closes (Close button or Escape inside the list). A source click opens the link dialog.
 * Depends on: AppContext, the transcript store hooks, SourcesList. Called by: Transcript.
 * Invariants: render only; the sources are the snapshot the chip handed over (no fetching).
 */
import { type ReactElement, useEffect, useRef } from "react";

import { useApp } from "../shell/AppContext";
import { useTranscriptUi } from "./hooks";
import { SourcesList } from "./rows/SourcesList";

/** The panel, while an answer's sources are shown. */
export function SourcesPanel(): ReactElement | null {
  const { transcript } = useApp();
  const sources = useTranscriptUi((s) => s.sources);
  const ref = useRef<HTMLElement>(null);
  const key = sources?.key ?? null;

  useEffect(() => {
    if (key === null) return;
    ref.current?.querySelector<HTMLElement>(".sources-close")?.focus({ preventScroll: true });
    return () => {
      // Back to the chip that opened it (it may have scrolled away: then nothing to focus).
      const chips = document.querySelectorAll<HTMLElement>("[data-sources-chip]");
      [...chips]
        .find((chip) => chip.dataset["sourcesChip"] === key)
        ?.focus({ preventScroll: true });
    };
  }, [key]);

  // A transcript that goes away (another session selected) closes its side list.
  useEffect(
    () => () => {
      transcript.showSources(null);
    },
    [transcript],
  );

  if (sources === null) return null;
  const close = () => {
    transcript.showSources(null);
  };
  return (
    <aside
      ref={ref}
      className="sources-panel"
      aria-labelledby="sources-panel-title"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          close();
        }
      }}
    >
      <header>
        <h2 id="sources-panel-title">Sources ({String(sources.items.length)})</h2>
        <button
          type="button"
          className="mini sources-close"
          aria-label="Close sources (Escape)"
          title="Close sources (Escape)"
          onClick={close}
        >
          Close
        </button>
      </header>
      <p className="d">
        Pages Mistral&apos;s web search used for this answer. Vibe Work does not open them; a click
        shows the full address first.
      </p>
      <SourcesList sources={sources.items} label="Web sources of this answer" />
    </aside>
  );
}
