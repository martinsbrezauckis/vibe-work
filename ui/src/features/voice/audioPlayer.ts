/**
 * Narration playback: plays one WAV at a time through Web Audio (`decodeAudioData` + a buffer
 * source). Not an `<audio>` element: the CSP has no `media-src`, so it falls back to `'self'` and would
 * refuse `data:`/`blob:` audio; Web Audio needs no CSP change. One player, one sound
 * at a time: playing a new one stops the old.
 * Depends on: the Web Audio API. Called by: state/narration and state/voiceChat (through `Player`).
 * Invariants: `play` resolves when the sound ended or was stopped, never rejects for a stop; it rejects
 * only when the audio cannot be decoded or played, and then always within a bounded time: an output that
 * cannot start (WebKitGTK without GStreamer's audio elements rejects `resume()` and never answers
 * `decodeAudioData`) or whose clock never moves rejects with a `PlaybackError` instead of waiting forever,
 * so no caller stays "playing". `stop()` is safe at any time.
 */

import { playbackOutput } from "./playbackGate";

/** What a finished playback reports. */
export interface Played {
  /** Decoded length in seconds. */
  seconds: number;
  /** True when `stop` cut it short. */
  stopped: boolean;
}

/** A player (tests pass a fake). */
export interface Player {
  play(audio: Uint8Array, onStarted?: () => void): Promise<Played>;
  stop(): void;
}

/** The sound could not play; the message is a lower-case reason that callers put after their own words. */
export class PlaybackError extends Error {}

/** How long each step may take before the sound counts as unplayable. */
export interface PlayerLimits {
  /** Starting the output: `resume()`, and the clock's first move after `start()`. */
  startMs: number;
  /** Decoding the WAV. */
  decodeMs: number;
}

const LIMITS: PlayerLimits = { startMs: 4_000, decodeMs: 15_000 };
/** How often the clock is read while a sound plays. */
const WATCH_MS = 250;
/** A clock this far past the sound's end means the end event was missed: the sound is over. */
const END_SLACK_S = 1;

/** `promise`, or `undefined` once `ms` passed (a rejection also gives `undefined`). */
function within<T>(promise: Promise<T>, ms: number): Promise<T | undefined> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      resolve(undefined);
    }, ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      () => {
        clearTimeout(timer);
        resolve(undefined);
      },
    );
  });
}

/** The Web Audio player. */
export function createWebAudioPlayer(limits: PlayerLimits = LIMITS): Player {
  let context: AudioContext | null = null;
  let current: { source: AudioBufferSourceNode; finish: (stopped: boolean) => void } | null = null;
  let generation = 0;
  const stop = (): void => {
    generation += 1;
    const playing = current;
    current = null;
    if (playing === null) return;
    try {
      playing.source.stop();
    } catch {
      // Already stopped.
    }
    playing.finish(true);
  };
  return {
    async play(audio, onStarted) {
      stop();
      const mine = generation;
      context ??= new AudioContext();
      const ctx = context;
      if (ctx.state !== "running") {
        const resumed = await within(
          ctx.resume().then(() => true),
          limits.startMs,
        );
        if (mine !== generation) return { seconds: 0, stopped: true };
        // Read again: `resume()` changed it (TypeScript still narrows the earlier read).
        if (resumed !== true || (ctx.state as AudioContextState) !== "running")
          throw new PlaybackError("no audio output is available");
      }
      const copy = audio.slice().buffer;
      const buffer = await within(ctx.decodeAudioData(copy), limits.decodeMs);
      if (mine !== generation) return { seconds: buffer?.duration ?? 0, stopped: true };
      if (buffer === undefined) throw new PlaybackError("the sound could not be decoded");
      const source = ctx.createBufferSource();
      source.buffer = buffer;
      const output = playbackOutput(ctx);
      source.connect(output.node);
      return new Promise<Played>((resolve, reject) => {
        let done = false;
        const begin = ctx.currentTime;
        const startedAt = performance.now();
        // The clock is the only sign that sound is leaving: no move = no output, past the end = over.
        // (The callbacks below run only after `start()`, when `settle` and `finish` exist.)
        const watch = setInterval(
          () => {
            if (done) return;
            const now = ctx.currentTime;
            if (typeof now !== "number" || typeof begin !== "number") return;
            if (now >= begin + buffer.duration + END_SLACK_S) {
              finish(false);
            } else if (now <= begin && performance.now() - startedAt >= limits.startMs) {
              settle();
              try {
                source.stop();
              } catch {
                // Never started.
              }
              reject(new PlaybackError("the audio output did not start"));
            }
          },
          Math.min(WATCH_MS, limits.startMs),
        );
        const settle = () => {
          done = true;
          clearInterval(watch);
          output.release();
          if (current?.source === source) current = null;
        };
        const finish = (stopped: boolean) => {
          if (done) return;
          settle();
          resolve({ seconds: buffer.duration, stopped });
        };
        current = { source, finish };
        source.onended = () => {
          finish(false);
        };
        source.start();
        onStarted?.();
      });
    },
    stop,
  };
}

/**
 * `player`, but each sound first asks the host whether sound can leave at all (`available`). In the Linux AppImage
 * without a sound server GStreamer plays into a silent fake sink, so the page itself cannot tell; then the sound
 * fails with "no audio output is available" instead of playing into nothing. A question that fails or has no
 * answer counts as available: the check must never block sound that would work.
 */
export function withOutputCheck(player: Player, available: () => Promise<boolean>): Player {
  return {
    async play(audio, onStarted) {
      const answer = await available().catch(() => true);
      if (!answer) {
        player.stop();
        throw new PlaybackError("no audio output is available");
      }
      return player.play(audio, onStarted);
    },
    stop: () => {
      player.stop();
    },
  };
}
