/**
 * Long plain text with a "Show all" switch (a 10 000-line tool output must not freeze the
 * window; Vibe's live output has no `truncated` flag, so the cut is ours). The cut text is
 * rendered at first; the full text only when the user asks, and the choice is remembered in the
 * transcript store (rows unmount when scrolled away).
 * Depends on: model/text, transcript store hooks. Called by: step bodies, the approval card.
 * Invariants: text only (a `<pre>`; with `linkify`, file paths become File tab buttons once the core said yes);
 * the switch says how much is hidden.
 */
import type { ReactElement } from "react";

import { LinkedText } from "../../files/FileRef";
import { useApp } from "../../shell/AppContext";
import { headText, tailText } from "../model/text";
import { useTranscriptUi } from "../hooks";

/** Default cut: lines and characters. */
export const CUT_LINES = 200;
export const CUT_CHARS = 20_000;

/** Props of [`ShowAll`]. */
export interface ShowAllProps {
  /** Store key for "shown in full" (unique per text). */
  id: string;
  text: string;
  /** Keep the end (logs) instead of the start (files). */
  tail?: boolean;
  className?: string;
  /** Make file paths in the text clickable (tool output). */
  linkify?: boolean;
}

/** Renders `text`, cut until the user asks for all of it. */
export function ShowAll({
  id,
  text,
  tail = false,
  className = "out",
  linkify = false,
}: ShowAllProps): ReactElement {
  const { transcript } = useApp();
  const key = `all:${id}`;
  const full = useTranscriptUi((s) => s.open[key] === true);
  const cut = tail ? tailText(text, CUT_LINES, CUT_CHARS) : headText(text, CUT_LINES, CUT_CHARS);
  const shown = full || !cut.cut ? text : cut.text;
  const hidden =
    cut.hiddenLines > 0
      ? `${String(cut.hiddenLines)} more lines`
      : `${String(text.length - cut.text.length)} more characters`;
  const more = (where: string) => (
    <button
      type="button"
      className="show-all-btn"
      aria-label="Show full text"
      title="Show full text"
      onClick={() => {
        transcript.toggle(key, true);
      }}
    >
      Show all ({hidden}
      {where})
    </button>
  );
  return (
    <div className="show-all">
      {cut.cut && !full && tail ? more(" above") : null}
      <pre className={className}>{linkify ? <LinkedText text={shown} /> : shown}</pre>
      {cut.cut && !full && !tail ? more("") : null}
      {cut.cut && full ? (
        <button
          type="button"
          className="show-all-btn"
          aria-label="Show less text"
          title="Show less text"
          onClick={() => {
            transcript.toggle(key, false);
          }}
        >
          Show less
        </button>
      ) : null}
    </div>
  );
}
