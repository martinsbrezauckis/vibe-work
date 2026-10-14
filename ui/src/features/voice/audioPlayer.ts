/**
 * Narration playback: plays one WAV at a time through Web Audio (`decodeAudioData` + a buffer
 * source). Not an `<audio>` element: the CSP has no `media-src`, so it falls back to `'self'` and would
 * refuse `data:`/`blob:` audio; Web Audio needs no CSP change. One player, one sound
 * at a time: playing a new one stops the old.
 * Depends on: the Web Audio API. Called by: state/narration (through `Player`).
 * Invariants: `play` resolves when the sound ended or was stopped, never rejects for a stop; it rejects
 * only when the audio cannot be decoded or played; `stop()` is safe at any time.
 */

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

/** The Web Audio player. */
export function createWebAudioPlayer(): Player {
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
      if (context.state === "suspended") await context.resume().catch(() => undefined);
      if (mine !== generation) return { seconds: 0, stopped: true };
      const copy = audio.slice().buffer;
      const buffer = await context.decodeAudioData(copy);
      if (mine !== generation) return { seconds: buffer.duration, stopped: true };
      const source = context.createBufferSource();
      source.buffer = buffer;
      source.connect(context.destination);
      return new Promise<Played>((resolve) => {
        let done = false;
        const finish = (stopped: boolean) => {
          if (done) return;
          done = true;
          if (current?.source === source) current = null;
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
