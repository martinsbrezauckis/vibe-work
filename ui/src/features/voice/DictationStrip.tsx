/**
 * The line above the text box while dictating: the timer, the live text (realtime), a note
 * (live text lost, the length limit, nothing heard) or the error, with Stop and Cancel. While the OS asks for the
 * microphone (a first use on macOS) it says so, with Cancel and no time limit.
 * Depends on: AppContext, the voice hooks. Called by: Composer.
 * Invariants: render only; shown for the session that dictates; the live text is plain text (React
 * escapes it), never Markdown.
 */
import type { ReactElement } from "react";

import { useApp } from "../shell/AppContext";
import { clock, useDictation } from "./hooks";

/** The strip for `sessionId`, or nothing. */
export function DictationStrip({ sessionId }: { sessionId: string }): ReactElement | null {
  const voice = useApp().voice;
  const phase = useDictation((s) => s.phase);
  const owner = useDictation((s) => s.sessionId);
  const seconds = useDictation((s) => s.seconds);
  const max = useDictation((s) => s.maxSeconds);
  const partial = useDictation((s) => s.partial);
  const mode = useDictation((s) => s.mode);
  const note = useDictation((s) => s.note);
  const error = useDictation((s) => s.error);
  const permissionWait = useDictation((s) => s.permissionWait);
  if (voice === null) return null;
  const { dictation } = voice;
  const live = phase === "starting" || phase === "recording" || phase === "transcribing";
  if (!live && note === null && error === null) return null;
  if (live && owner !== sessionId) return null;
  return (
    <div className={`dictation dictation-${phase}`} role="status" data-testid="dictation-strip">
      {phase === "recording" ? <span className="rec-dot" aria-hidden="true" /> : null}
      <span className="mono dictation-time">
        {phase === "recording" ? `${clock(seconds)} / ${clock(max)}` : null}
      </span>
      <span className="dictation-text">
        {phase === "starting"
          ? permissionWait
            ? "Waiting for microphone permission…"
            : "Starting the microphone…"
          : phase === "transcribing"
            ? partial === ""
              ? "Making the text…"
              : partial
            : phase === "recording"
              ? partial === ""
                ? mode === "batch"
                  ? "Listening. The text comes when you stop."
                  : "Listening…"
                : partial
              : (error ?? note)}
      </span>
      {phase === "recording" ? (
        <button
          type="button"
          className="mini"
          title="Stop dictation and transcribe speech"
          aria-label="Stop dictation and transcribe speech"
          onClick={() => void dictation.stop()}
        >
          Stop
        </button>
      ) : null}
      {phase === "recording" || phase === "starting" ? (
        <button
          type="button"
          className="mini"
          title="Cancel dictation (Escape)"
          aria-label="Cancel dictation (Escape)"
          onClick={() => {
            dictation.cancel();
          }}
        >
          Cancel
        </button>
      ) : null}
      {!live ? (
        <button
          type="button"
          className="mini"
          aria-label="Dismiss dictation notice"
          title="Dismiss dictation notice"
          onClick={() => {
            dictation.dismiss();
          }}
        >
          OK
        </button>
      ) : null}
    </div>
  );
}
