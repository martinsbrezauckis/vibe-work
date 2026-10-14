/**
 * The session row's voice chat mark: on the owner session's row, a waveform icon whose colour (and pulse
 * while listening) shows what voice chat is doing; the words (listening, thinking, speaking, waiting, error) are its
 * label and tooltip. Icon only: a word tag squeezed the title to one letter at 1280 px (e2e screenshot).
 * Depends on: the voice hooks, Icon. Called by: sessions/SessionRow.
 * Invariants: render only; nothing for any other session.
 */
import type { ReactElement } from "react";

import { Icon } from "../../components/Icon";
import type { VoiceChatPhase } from "../../state/voiceChat";
import { useVoiceChat } from "./hooks";

/** The tag's word per phase. */
const WORD: Readonly<Record<VoiceChatPhase, string>> = {
  off: "",
  listening: "listening",
  sending: "sending",
  thinking: "thinking",
  waiting: "waiting",
  speaking: "speaking",
  error: "voice error",
};

/** The tag for `sessionId`'s row, or nothing. */
export function VoiceChatTag({ sessionId }: { sessionId: string }): ReactElement | null {
  const phase = useVoiceChat((s) => (s.on && s.owner === sessionId ? s.phase : "off"));
  if (phase === "off") return null;
  return (
    <span
      className={`vchat-row vchat-row-${phase}`}
      title={`Voice chat: ${WORD[phase]}`}
      role="img"
      aria-label={`Voice chat: ${WORD[phase]}`}
      data-testid="voice-chat-row-tag"
      data-phase={phase}
    >
      <Icon name="audio-lines" size={14} />
    </span>
  );
}
