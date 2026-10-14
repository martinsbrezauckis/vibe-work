/**
 * The Voice chat button: the composer slot's face while the box is empty, and the only face while the
 * session is in voice chat (no Send then). Its look and label follow the loop: off, listening (press = end my turn),
 * sending, thinking, waiting for the card, speaking (press = stop the reply and talk), error (press = try again).
 * Voice chat is turned off with End on the strip above the text box (`VoiceChatStrip`); ShellX `BottomPanel.tsx`
 * @ 58a9ddac had the same pair ("Voice chat · ON" + an off chip).
 * Depends on: AppContext, the voice hooks, Icon, Walker. Called by: Composer (through the tool row's slot).
 * Invariants: render and event wiring only (the controller decides); while a dictation runs the button is disabled
 * (one microphone); labels say what a press does.
 */
import type { ReactElement } from "react";

import { Icon } from "../../components/Icon";
import { Walker } from "../../components/Walker";
import type { VoiceChatPhase } from "../../state/voiceChat";
import { useApp } from "../shell/AppContext";
import { useKeyReadiness } from "../shell/KeyNotice";
import { useDictation, useVoiceChat, useVoiceSettings } from "./hooks";

/** What the button says per phase (`auto`: the pause also ends the turn). */
function label(phase: VoiceChatPhase, auto: boolean, error: string | null): string {
  switch (phase) {
    case "listening":
      return auto
        ? "Voice chat: Listening. Pause or press to send what you said"
        : "Voice chat: Listening. Press to send what you said";
    case "sending":
      return "Voice chat: sending what you said; wait for Vibe";
    case "thinking":
      return "Voice chat: Vibe is working; wait for the spoken answer";
    case "waiting":
      return "Voice chat: answer the waiting card in the chat";
    case "speaking":
      return "Voice chat: stop reading the answer aloud and talk";
    case "error":
      return `Voice chat: ${error ?? "something went wrong"}. Press to try again`;
    default:
      return "Voice chat: talk with Vibe and hear the answers";
  }
}

/** The button for `sessionId`'s composer. */
export function VoiceChatButton({ sessionId }: { sessionId: string }): ReactElement | null {
  const voice = useApp().voice;
  const keyBlocked = useKeyReadiness() !== null;
  const on = useVoiceChat((s) => s.on && s.owner === sessionId);
  const phase = useVoiceChat((s) => (s.on && s.owner === sessionId ? s.phase : "off"));
  const error = useVoiceChat((s) => s.error);
  const auto = useVoiceSettings((s) => s.voiceChatAutoEnd);
  const dictating = useDictation(
    (s) => s.phase === "starting" || s.phase === "recording" || s.phase === "transcribing",
  );
  if (voice === null) return null;
  const blocked = !on && (keyBlocked || dictating);
  const text = keyBlocked
    ? "Voice chat needs a usable Mistral key. Open Voice settings to restore it."
    : blocked
      ? "Voice chat: finish the dictation first"
      : label(phase, auto, error);
  return (
    <button
      type="button"
      className={`vchat vchat-${phase}`}
      aria-label={text}
      title={text}
      aria-pressed={on}
      disabled={blocked}
      data-testid="voice-chat-button"
      data-phase={phase}
      onClick={() => {
        if (keyBlocked) voice.voiceChat.turnOff();
        else voice.voiceChat.press(sessionId);
      }}
    >
      <Icon name={phase === "speaking" ? "square" : "audio-lines"} />
      {phase === "thinking" || phase === "sending" ? <Walker size="badge" /> : null}
    </button>
  );
}
