/**
 * The wait for an opened microphone's first sound, for voice chat. Opening the microphone (`getUserMedia`
 * resolving) does not prove that it works: a muted microphone, a wrongly routed device or a capture the system
 * never started sends only digital silence (every sample exactly zero) or nothing. A working microphone sends its
 * noise floor within a block, so "no sample other than zero for `NO_SOUND_MS`" means the person cannot be heard.
 * Depends on: turnEnd (`pcmRms`). Called by: state/voiceChat.
 * A delivered chunk with sound ends the wait at once (`heard`), before the core acknowledges it: one short word
 * whose append is still in flight is sound, not silence.
 * Invariants: at most one wait runs; `clear` is idempotent; the callback runs at most once per `arm`, never for a
 * stream that delivered sound, and only when `stillWaiting` (the caller's own generation and stream check) says the
 * same listening stretch still waits.
 */
import { pcmRms } from "../features/voice/turnEnd";
import { NO_SOUND_MS } from "./voiceChatState";

/** Why voice chat stopped when the microphone sent no sound (a warning toast, kept in Notifications). */
export const NO_SOUND_REASON =
  "No sound came from the microphone, so voice chat stopped. Check that the right microphone is chosen in Settings > Voice and that it is not muted, then press Voice chat again.";

/** True when a 16-bit PCM chunk holds any sample other than exact digital silence. */
export const hasSound = (pcm: Uint8Array): boolean => pcmRms(pcm) > 0;

/** Creates the wait; `arm` when the microphone opened, `clear` on the first sound or when listening stops. */
export function createSoundWait() {
  let timer: ReturnType<typeof setTimeout> | null = null;
  // The stream (a listening stretch's id) whose microphone delivered sound.
  let delivered: string | null = null;
  const clear = (): void => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };
  return {
    clear,
    /** True when `pcm` holds sound; then stream `id`'s microphone works and its wait ends (or never starts). */
    heard(id: string, pcm: Uint8Array): boolean {
      if (!hasSound(pcm)) return false;
      delivered = id;
      clear();
      return true;
    },
    /** Starts stream `id`'s wait; `stop` gets `NO_SOUND_REASON` if no sound came and `stillWaiting` still holds. */
    arm(id: string, stillWaiting: () => boolean, stop: (reason: string) => void): void {
      clear();
      if (delivered === id) return;
      timer = setTimeout(() => {
        timer = null;
        if (delivered !== id && stillWaiting()) stop(NO_SOUND_REASON);
      }, NO_SOUND_MS);
    },
  };
}
