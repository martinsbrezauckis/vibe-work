/**
 * Speech that arrives after a voice turn ended, through sending and thinking until reply playback starts: the
 * microphone stays open, and the next listening stretch starts with the kept audio.
 * Depends on: nothing. Called by: state/voiceChat.
 * Invariants: bounded (the newest `MAX_CHUNKS` chunks); kept only when some chunk was louder than the turn-end
 * threshold, otherwise `take` returns nothing (room noise is not carried into the next message).
 */

/** Newest chunks kept (~60 s of 250 ms chunks). */
const MAX_CHUNKS = 240;
/** Said after reply playback when speech was cut off by that playback. */
export const CARRY_CUTOFF =
  "I stopped listening while Vibe answered. Go on from where I cut you off.";
/** A bounded carry dropped some earlier speech. */
export const CARRY_OVERFLOW =
  "Some earlier words could not be kept. Please say them again when listening resumes.";

/** Creates an empty carry. */
export function createCarry() {
  let chunks: { pcm: Uint8Array; loud: boolean }[] = [];
  let loudCount = 0;
  let overflowed = false;
  let threshold = 0.01;
  let quietMs = 0;
  let cutoffPending = false;
  return {
    /** Starts collecting after a turn ended; `above` is the RMS that counted as speech in that turn. */
    arm(above: number): void {
      chunks = [];
      loudCount = 0;
      overflowed = false;
      threshold = above;
      quietMs = 0;
      cutoffPending = false;
    },
    /** Keeps one chunk; returns true once when earlier speech is lost to the bound. */
    keep(pcm: Uint8Array, rms: number, sampleRate: number): boolean {
      const loud = rms >= threshold;
      chunks.push({ pcm, loud });
      if (loud) loudCount++;
      quietMs = loud ? 0 : quietMs + (pcm.length / 2 / sampleRate) * 1000;
      if (chunks.length > MAX_CHUNKS) {
        const removed = chunks.shift();
        if (removed?.loud) {
          loudCount--;
          if (!overflowed) {
            overflowed = true;
            return true;
          }
        }
      }
      return false;
    },
    /** Whether the retained audio includes speech. */
    hasSpeech(): boolean {
      return loudCount > 0;
    },
    /** Remembers that playback interrupted speech before a full configured pause. */
    markCutoff(pauseMs: number): void {
      if (loudCount > 0 && quietMs < pauseMs) cutoffPending = true;
    },
    /** One notice after the answer, before listening resumes. */
    takeCutoffNotice(): string | null {
      if (!cutoffPending) return null;
      cutoffPending = false;
      return CARRY_CUTOFF;
    },
    /** The carried audio when it holds speech, else nothing; always empties the carry. */
    take(): Uint8Array[] {
      const out = loudCount > 0 ? chunks.map((chunk) => chunk.pcm) : [];
      chunks = [];
      loudCount = 0;
      overflowed = false;
      quietMs = 0;
      cutoffPending = false;
      return out;
    },
  };
}
