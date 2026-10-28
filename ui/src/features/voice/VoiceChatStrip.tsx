/**
 * The line above the text box while the session is in voice chat: what the loop is doing (listening with
 * the live words, sending, thinking, waiting for the card, reading the answer), a note ("Nothing was heard") or the
 * error that kept the mode on, and End.
 * Depends on: AppContext, the voice hooks, the announcer. Called by: Composer.
 * Invariants: render only; shown for the owner session only; live words and errors are plain text (React escapes
 * them), never Markdown. The strip is not a live region: an error is said once by the app's announcer.
 */
import { Badge, Button } from "../../components/Controls";
import { type ReactElement, useEffect } from "react";
import { announce } from "../../state/announcer";

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
  // Said once: an error. Not the loop's phases or the live words (Vibe Work's own voice is talking with the person,
  // and the microphone would hear the screen reader).
  const said = mine && error !== null ? error : "";
  useEffect(() => {
    if (said !== "") announce(said, { assertive: true });
  }, [said]);
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
      data-testid="voice-chat-strip"
    >
      {phase === "listening" && !micPending ? (
        <span className="rec-dot" aria-hidden="true" />
      ) : null}
      <Badge presentation="voice-strip">Voice chat</Badge>
      <span className="dictation-text">
        {text}
        {extra === null ? null : <span className="vchat-note"> {extra}</span>}
      </span>
      <Button
        data-testid="voice.voice-chat-strip-turn-off"
        type="button"
        appearance="compact"
        title="End voice chat"
        aria-label="End voice chat"
        onClick={() => {
          voice.voiceChat.turnOff();
        }}
      >
        End
      </Button>
    </div>
  );
}
