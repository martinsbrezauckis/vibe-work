/**
 * Settings › Voice › Voice chat: "End my turn after a pause" (on by default) and the pause length, plus
 * what voice chat sends where (whether a Vibe plan's included usage covers speech is not confirmed,
 * so the line says the usage goes to the user's key).
 * Depends on: AppContext, the voice hooks, turnEnd (range). Called by: settings/VoiceSettings.
 * Invariants: changing a choice never starts anything; the pause is always within 0.8-2.5 s (`voiceSettings` checks).
 */
import type { ReactElement } from "react";

import { Selector } from "../../components/Selector";
import { useApp } from "../shell/AppContext";
import { KeyNotice } from "../shell/KeyNotice";
import { useVoiceSettings } from "../voice/hooks";
import { PAUSE_MAX_MS, PAUSE_MIN_MS } from "../voice/turnEnd";

/** The pause lengths offered (ms). */
const PAUSES = [800, 1000, 1200, 1500, 2000, 2500] as const;

/** Seconds as the list shows them ("1.2 s"). */
function seconds(ms: number): string {
  return `${(ms / 1000).toFixed(1)} s`;
}

/** The voice chat block of the Voice settings (nothing without a core). */
export function VoiceChatSettings(): ReactElement | null {
  const { voice } = useApp();
  const auto = useVoiceSettings((s) => s.voiceChatAutoEnd);
  const pause = useVoiceSettings((s) => s.voiceChatPauseMs);
  if (voice === null) return null;
  const offered: readonly number[] = PAUSES.includes(pause as (typeof PAUSES)[number])
    ? PAUSES
    : [...PAUSES, pause].sort((a, b) => a - b);
  return (
    <>
      <h4 id="voice-chat-title">Voice chat</h4>
      <p className="key-field-hint ui-help">
        The Voice chat button next to the microphone: you talk, Vibe answers briefly and the answer
        is read aloud with the voice above, then it listens again. What you say goes to Mistral
        speech-to-text and the answers to Mistral text-to-speech, with your key and on your usage.
        Audio is not saved.
      </p>
      <KeyNotice
        inSettings
        message="Voice chat needs your Mistral key to listen and read replies aloud."
      />
      <div className="check" data-setting="voice-chat-auto-end">
        <span>
          End my turn after a pause
          <span className="d">
            When you stop talking for the time below, what you said is sent. Off: press the voice
            button to send.
          </span>
        </span>
        <button
          data-testid="settings.voice-chat-settings-update"
          type="button"
          className="switch"
          role="switch"
          aria-label="End my turn after a pause"
          aria-checked={auto}
          onClick={() => {
            voice.settings.update({ voiceChatAutoEnd: !auto });
          }}
        />
      </div>
      <div className="check" data-setting="voice-chat-pause">
        <span>
          Pause before sending
          <span className="d">
            {`Between ${seconds(PAUSE_MIN_MS)} and ${seconds(PAUSE_MAX_MS)}. Longer if you think aloud.`}
          </span>
        </span>
        <Selector
          testId="settings.voice-chat-settings-update-2"
          label="Pause before sending"
          value={String(pause)}
          disabled={!auto}
          options={offered.map((ms) => ({ value: String(ms), label: seconds(ms) }))}
          onChange={(value) => {
            voice.settings.update({ voiceChatPauseMs: Number(value) });
          }}
        />
      </div>
    </>
  );
}
