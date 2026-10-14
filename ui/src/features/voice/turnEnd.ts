/**
 * Automatic end of the user's turn in voice chat (the "easy
 * version"): the turn ends after a pause, when the audio energy stayed below a threshold
 * AND no new realtime transcript words came for `pauseMs` (including quiet speech below the energy threshold).
 * No model-based turn detection.
 * The threshold adapts to the room: it is the larger of a fixed floor and `NOISE_RATIO` × the noise floor, which
 * the first chunk sets and then follows the quietest chunks (drops at once, rises slowly), so steady background noise
 * does not count as speech.
 * The rule arms only once speech was heard (`MIN_SPEECH_MS` above the threshold, or a transcript word), so a user
 * who has not started talking is never cut off.
 * Depends on: nothing. Called by: state/voiceChat (one detector per listening stretch), tests (synthetic PCM).
 * Invariants: pure apart from its own state; time comes in as arguments (ms), never from a clock.
 */

/** Shortest and longest pause a user can choose (the Settings slider). */
export const PAUSE_MIN_MS = 800;
export const PAUSE_MAX_MS = 2_500;
/** The default pause. */
export const PAUSE_DEFAULT_MS = 800;

/** RMS (0..1) below which a chunk is quiet in any room. */
const FLOOR_RMS = 0.01;
/** Speech must be this many times louder than the noise floor. */
const NOISE_RATIO = 3;
/** Speech heard (above the threshold) before a pause can end the turn. */
const MIN_SPEECH_MS = 300;
/** How fast the noise floor rises towards louder chunks (share of the gap per chunk of ~250 ms). */
const RISE = 0.03;

/** RMS (0..1) of 16-bit little-endian PCM. */
export function pcmRms(pcm: Uint8Array): number {
  const n = Math.floor(pcm.length / 2);
  if (n === 0) return 0;
  const view = new DataView(pcm.buffer, pcm.byteOffset, n * 2);
  let sum = 0;
  for (let i = 0; i < n; i += 1) {
    const s = view.getInt16(i * 2, true) / 0x8000;
    sum += s * s;
  }
  return Math.sqrt(sum / n);
}

/** What the detector reports to the state (and the Debug API). */
export interface TurnEndView {
  /** Speech was heard (the rule is armed). */
  heard: boolean;
  /** The current threshold (RMS). */
  threshold: number;
}

/**
 * Creates a detector. `words` = the stream gives live transcript words (realtime mode); without them (batch mode,
 * a fixed language) only the energy part of the rule applies.
 */
export function createTurnEnd(options: { pauseMs: number; words: boolean }) {
  const pause = Math.min(PAUSE_MAX_MS, Math.max(PAUSE_MIN_MS, options.pauseMs));
  let noise: number | null = null;
  let speechMs = 0;
  let lastLoud: number | null = null;
  let lastWords: number | null = null;
  let heardWords = false;
  const threshold = (): number => Math.max(FLOOR_RMS, (noise ?? 0) * NOISE_RATIO);
  return {
    /** One chunk of audio of `durationMs` with energy `rms`, received at `now`. */
    audio(rms: number, durationMs: number, now: number): void {
      // The first chunk (the microphone just opened, before the user talks) sets the noise floor and is not judged:
      // steady room noise then never counts as speech. Afterwards the floor falls at once to a quieter chunk and
      // rises by `RISE` of the gap per chunk: speech has quieter chunks between words, so a floor raised by speech
      // (or set by a user who talked from the first instant) comes back down at the next breath.
      if (noise === null) {
        noise = rms;
        return;
      }
      if (rms >= threshold() && rms > FLOOR_RMS) {
        speechMs += durationMs;
        lastLoud = now;
      }
      noise = rms < noise ? rms : noise + (rms - noise) * RISE;
    },
    /** The live transcript changed at `now` (new words). */
    words(now: number): void {
      lastWords = now;
      heardWords = true;
    },
    /** True when the user's turn should end now. */
    ended(now: number): boolean {
      const heard = speechMs >= MIN_SPEECH_MS || heardWords;
      if (!heard) return false;
      const quietFor = lastLoud === null ? Number.POSITIVE_INFINITY : now - lastLoud;
      if (quietFor < pause) return false;
      if (!options.words) return true;
      return lastWords === null || now - lastWords >= pause;
    },
    /** The detector's state for views. */
    view(): TurnEndView {
      return { heard: speechMs >= MIN_SPEECH_MS || heardWords, threshold: threshold() };
    },
  };
}

/** A turn-end detector. */
export type TurnEnd = ReturnType<typeof createTurnEnd>;
