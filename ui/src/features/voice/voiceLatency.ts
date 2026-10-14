/**
 * Voice turn timings, in epoch milliseconds on the UI clock. Read by the Debug API latency harness.
 * Depends on: nothing. Called by: voiceChat. No text, audio or credentials are recorded here.
 */
export interface VoiceLatency {
  endSpeech: number;
  endDetected: number;
  transcriptionDone: number | null;
  turnSent: number | null;
  firstModelToken: number | null;
  firstSentence: number | null;
  firstTtsRequest: number | null;
  firstAudioPlayed: number | null;
  turnEnd: number | null;
}

/** A new trace starts when capture ends; endSpeech is the last chunk above the detector's threshold. */
export function voiceLatency(endSpeech: number, endDetected: number): VoiceLatency {
  return {
    endSpeech,
    endDetected,
    transcriptionDone: null,
    turnSent: null,
    firstModelToken: null,
    firstSentence: null,
    firstTtsRequest: null,
    firstAudioPlayed: null,
    turnEnd: null,
  };
}
