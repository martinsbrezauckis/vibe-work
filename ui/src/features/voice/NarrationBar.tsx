/**
 * The narration bar: what is being read aloud (Vibe's one-line summary of the turn), with
 * Skip (the next queued turn plays) and Stop (silence, queue cleared); a short note when a turn was not
 * read (dictating, no summary, a failure).
 * Depends on: AppContext, the voice hooks, Icon. Called by: Composer.
 * Invariants: render only; shown only while narration works or has a note; the summary is plain text.
 */
import type { ReactElement } from "react";

import { Icon } from "../../components/Icon";
import { useApp } from "../shell/AppContext";
import { useNarration } from "./hooks";

/** The bar, or nothing. */
export function NarrationBar(): ReactElement | null {
  const voice = useApp().voice;
  const phase = useNarration((s) => s.phase);
  const summary = useNarration((s) => s.current?.summary ?? null);
  const waiting = useNarration((s) => s.queue.length);
  const note = useNarration((s) => s.note);
  if (voice === null || (phase === "idle" && note === null)) return null;
  const { narration } = voice;
  const text =
    phase === "idle"
      ? note
      : phase === "summarizing"
        ? "Summarizing the turn…"
        : (summary ?? "Reading aloud…");
  return (
    <div className="narration" role="status" data-testid="narration-bar">
      <Icon name="volume-2" size={14} />
      <span className="narration-text">{text}</span>
      {waiting > 0 ? <span className="mono">+{waiting}</span> : null}
      {phase !== "idle" ? (
        <>
          <button
            type="button"
            className="ibtn"
            aria-label="Skip this narration"
            title="Skip this narration"
            onClick={() => {
              narration.skip();
            }}
          >
            <Icon name="skip-forward" size={14} />
          </button>
          <button
            type="button"
            className="ibtn"
            aria-label="Stop narration and clear queue"
            title="Stop narration and clear queue"
            onClick={() => {
              narration.stopAll();
            }}
          >
            <Icon name="square" size={14} />
          </button>
        </>
      ) : null}
    </div>
  );
}
