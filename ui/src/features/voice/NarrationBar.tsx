/**
 * The narration bar: what is being read aloud (Vibe's one-line summary of the turn), with
 * Skip (the next queued turn plays) and Stop (silence, queue cleared); a short note when a turn was not
 * read (dictating, no summary, a failure).
 * Depends on: AppContext, the voice hooks, Icon. Called by: Composer.
 * Invariants: render only; shown only while narration works or has a note; the summary is plain text.
 */
import type { ReactElement } from "react";

import { IconButton } from "../../components/IconButton";
import { Icon } from "../../components/Icon";
import { useApp } from "../shell/AppContext";
import { useNarration } from "./hooks";
import { useShellState } from "../shell/hooks";

/** The bar, or nothing. */
export function NarrationBar(): ReactElement | null {
  const app = useApp();
  const voice = app.voice;
  const current = useNarration((s) => s.current);
  const selected = useShellState((s) => s.selected);
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
        : current?.reading !== undefined
          ? selected === current.sessionId
            ? `Reading aloud: ${summary ?? ""}`
            : `Reading aloud from "${current.reading.title}": ${summary ?? ""}`
          : (summary ?? "Reading aloud…");
  return (
    <div className="narration" role="status" data-testid="narration-bar">
      <Icon name="volume-2" size={14} />
      <span className="narration-text">{text}</span>
      {waiting > 0 ? <span className="mono">+{waiting}</span> : null}
      {phase !== "idle" ? (
        <>
          <IconButton
            data-testid="voice.narration-bar-skip"
            type="button"
            label="Skip this narration"
            title="Skip this narration"
            onClick={() => {
              narration.skip();
            }}
            icon="skip-forward"
            iconSize={14}
          />
          <IconButton
            data-testid="voice.narration-bar-stop-all"
            type="button"
            label="Stop narration and clear queue"
            title="Stop narration and clear queue"
            onClick={() => {
              narration.stopAll();
            }}
            icon="square"
            iconSize={14}
          />
        </>
      ) : null}
    </div>
  );
}
