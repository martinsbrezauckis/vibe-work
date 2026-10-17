/**
 * The line above the text box while the session is in voice chat: what the loop is doing (listening with
 * the live words, sending, thinking, waiting for the card, reading the answer), a note ("Nothing was heard") or the
 * error that kept the mode on, and End.
 * Depends on: AppContext, the voice hooks. Called by: Composer.
 * Invariants: render only; shown for the owner session only; live words and errors are plain text (React escapes
 * them), never Markdown.
 */
import type { ReactElement } from "react";

import { useApp } from "../shell/AppContext";
import { useVoiceChat, useVoiceSettings } from "./hooks";

/** The strip for `sessionId`, or nothing. */
export function VoiceChatStrip({ sessionId }: { sessionId: string }): ReactElement | null {
  const voice = useApp().voice;
  const mine = useVoiceChat((s) => s.on && s.owner === sessionId);
  const phase = useVoiceChat((s) => s.phase);
  const partial = useVoiceChat((s) => s.partial);
  const micPending = useVoiceChat((s) => s.micPending);
  const note = useVoiceChat((s) => s.note);
  const error = useVoiceChat((s) => s.error);
  const auto = useVoiceSettings((s) => s.voiceChatAutoEnd);
  if (voice === null || !mine) return null;
  const text =
    phase === "listening"
      ? micPending
        ? "Opening the microphone…"
        : partial !== ""
          ? partial
          : auto
            ? "Listening. Speak, then pause or press the voice button."
            : "Listening. Speak, then press the voice button."
      : phase === "sending"
        ? partial !== ""
          ? partial
          : "Sending what you said…"
        : phase === "thinking"
          ? "Vibe is working on it."
          : phase === "waiting"
            ? "Vibe is waiting for your answer on the card in the chat."
            : phase === "speaking"
              ? "Reading the answer aloud."
              : (error ?? "Voice chat stopped for a moment.");
  const extra = phase === "error" ? null : (error ?? note);
  return (
    <div
      className={`dictation vchat-strip vchat-strip-${phase}${phase === "error" ? " dictation-error" : ""}`}
      role="status"
      data-testid="voice-chat-strip"
    >
      {phase === "listening" && !micPending ? (
        <span className="rec-dot" aria-hidden="true" />
      ) : null}
      <span className="vchat-tag">Voice chat</span>
      <span className="dictation-text">
        {text}
        {extra === null ? null : <span className="vchat-note"> {extra}</span>}
      </span>
      <button
        data-testid="voice.voice-chat-strip-turn-off"
        type="button"
        className="mini"
        title="End voice chat"
        aria-label="End voice chat"
        onClick={() => {
          voice.voiceChat.turnOff();
        }}
      >
        End
      </button>
    </div>
  );
}
